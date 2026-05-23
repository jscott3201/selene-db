//! Expression lowering helpers.

use crate::{
    IsCheckKind, ProcedureRegistry, SourceSpan, ValueExpr,
    analyze::{AnalyzedStatement, BindingId, ExprId},
    plan::{
        AggregateArg, CatalogOp, ExecutionPlan, FilterPredicate, FilterPredicateKind, JoinTree,
        MutationOp, OrderKey, OuterBindingRef, PipelineOp, PlannedSubquery,
        PlannedTypePropertyConstraint, PlannerError, ProjectExpr, SubqueryBody, SubqueryKind,
    },
};

use super::match_clause;

pub(crate) use super::binding_refs::binding_refs_in;

/// Build a planned projection expression.
pub(crate) fn project_expr(
    expr: &ValueExpr,
    alias: Option<selene_core::IStr>,
    analyzed: &AnalyzedStatement,
) -> Result<ProjectExpr, PlannerError> {
    let (expr_id, ty) = expr_cell(expr, analyzed)?;
    Ok(ProjectExpr {
        expr: expr.clone(),
        expr_id,
        ty,
        alias,
        binding_refs: binding_refs_in(expr, analyzed)?,
        span: expr.span(),
    })
}

/// Build a planned filter predicate from a boolean expression.
pub(crate) fn filter_predicate(
    expr: &ValueExpr,
    analyzed: &AnalyzedStatement,
) -> Result<FilterPredicate, PlannerError> {
    let (expr_id, ty) = expr_cell(expr, analyzed)?;
    Ok(FilterPredicate {
        expr: expr.clone(),
        expr_id,
        ty,
        binding_refs: binding_refs_in(expr, analyzed)?,
        kind: FilterPredicateKind::Expression,
        index_consumed: false,
        span: expr.span(),
    })
}

/// Build a property-map equality predicate.
pub(crate) fn property_predicate(
    binding: Option<BindingId>,
    key: selene_core::IStr,
    value: &ValueExpr,
    analyzed: &AnalyzedStatement,
) -> Result<FilterPredicate, PlannerError> {
    let (expr_id, ty) = expr_cell(value, analyzed)?;
    let mut binding_refs = binding_refs_in(value, analyzed)?;
    if let Some(binding) = binding {
        ensure_binding_exists(binding, value.span(), analyzed)?;
        binding_refs.push(binding);
        binding_refs.sort();
        binding_refs.dedup();
    }
    Ok(FilterPredicate {
        expr: value.clone(),
        expr_id,
        ty,
        binding_refs,
        kind: FilterPredicateKind::PropertyEquals { binding, key },
        index_consumed: false,
        span: value.span(),
    })
}

/// Build a planned sort key.
pub(crate) fn order_key(
    term: &crate::OrderTerm,
    analyzed: &AnalyzedStatement,
) -> Result<OrderKey, PlannerError> {
    let (expr_id, ty) = expr_cell(&term.expr, analyzed)?;
    Ok(OrderKey {
        expr: term.expr.clone(),
        expr_id,
        ty,
        direction: term.direction,
        nulls: term.nulls,
        binding_refs: binding_refs_in(&term.expr, analyzed)?,
        access: None,
        span: term.span,
    })
}

/// Build a planned aggregate argument.
pub(crate) fn aggregate_arg(
    expr: &ValueExpr,
    analyzed: &AnalyzedStatement,
) -> Result<AggregateArg, PlannerError> {
    let (expr_id, ty) = expr_cell(expr, analyzed)?;
    Ok(AggregateArg {
        expr: expr.clone(),
        expr_id,
        ty,
    })
}

/// Populate the plan-level expression-subquery registry.
pub(crate) fn populate_plan_subqueries(
    plan: &mut ExecutionPlan,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
) -> Result<(), PlannerError> {
    let mut entries = Vec::new();
    if let Some(pattern) = plan.pattern_plan.as_mut() {
        collect_subqueries_in_pattern_plan(pattern, analyzed, registry, &mut entries)?;
    }
    for op in &mut plan.pipeline {
        collect_subqueries_in_pipeline_op(op, analyzed, registry, &mut entries)?;
    }
    for (expr_id, subquery) in entries {
        plan.subqueries.insert(expr_id, subquery);
    }
    Ok(())
}

/// Return analyzer expression cell data.
pub(crate) fn expr_cell(
    expr: &ValueExpr,
    analyzed: &AnalyzedStatement,
) -> Result<(ExprId, crate::AnalyzedType), PlannerError> {
    let expr_id = analyzed
        .expr_ids
        .get(expr)
        .ok_or(PlannerError::ExpressionTypeMissing { span: expr.span() })?;
    Ok((expr_id, analyzed.expr_types.get(expr_id).clone()))
}

fn ensure_binding_exists(
    binding: BindingId,
    span: SourceSpan,
    analyzed: &AnalyzedStatement,
) -> Result<(), PlannerError> {
    analyzed
        .scopes
        .declaration(binding)
        .map(|_| ())
        .ok_or(PlannerError::BindingResolutionLost { binding, span })
}

fn collect_subqueries_in_pipeline_op(
    op: &mut PipelineOp,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    match op {
        PipelineOp::Filter(predicate) => {
            collect_subqueries_in_expr(&predicate.expr, analyzed, registry, entries)?;
        }
        PipelineOp::Project(projects) | PipelineOp::Let(projects) => {
            for project in projects {
                collect_subqueries_in_project(project, analyzed, registry, entries)?;
            }
        }
        PipelineOp::Unwind { source, .. } => {
            collect_subqueries_in_project(source, analyzed, registry, entries)?;
        }
        PipelineOp::OrderBy(keys) | PipelineOp::TopK { keys, .. } => {
            for key in keys {
                collect_subqueries_in_expr(&key.expr, analyzed, registry, entries)?;
            }
        }
        PipelineOp::GroupBy { keys, aggregates } => {
            for key in keys {
                collect_subqueries_in_project(key, analyzed, registry, entries)?;
            }
            for aggregate in aggregates {
                for arg in &aggregate.args {
                    collect_subqueries_in_expr(&arg.expr, analyzed, registry, entries)?;
                }
            }
        }
        PipelineOp::Union { rhs, .. } | PipelineOp::Chain(rhs) => {
            populate_plan_subqueries(rhs, analyzed, registry)?;
        }
        PipelineOp::Match(pattern) => {
            collect_subqueries_in_pattern_plan(pattern, analyzed, registry, entries)?
        }
        PipelineOp::ExplainPlan { inner, .. } => {
            populate_plan_subqueries(inner, analyzed, registry)?
        }
        PipelineOp::Call(call) => {
            for arg in &call.args {
                collect_subqueries_in_project(arg, analyzed, registry, entries)?;
            }
        }
        PipelineOp::Mutation(op) => {
            collect_subqueries_in_mutation(op, analyzed, registry, entries)?
        }
        PipelineOp::Catalog(op) => collect_subqueries_in_catalog(op, analyzed, registry, entries)?,
        PipelineOp::Limit { .. } | PipelineOp::Distinct | PipelineOp::Tx(_) => {}
    }
    Ok(())
}

fn collect_subqueries_in_pattern_plan(
    pattern: &mut crate::PatternPlan,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    for filter in &pattern.filters {
        collect_subqueries_in_expr(&filter.expr, analyzed, registry, entries)?;
    }
    collect_subqueries_in_join_tree(&mut pattern.join_tree, analyzed, registry, entries)
}

fn collect_subqueries_in_join_tree(
    tree: &mut JoinTree,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    match tree {
        JoinTree::Scan(scan) => {
            for predicate in &scan.property_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, registry, entries)?;
            }
        }
        JoinTree::Expand { child, edge, .. } => {
            collect_subqueries_in_join_tree(child, analyzed, registry, entries)?;
            for predicate in &edge.property_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, registry, entries)?;
            }
            for predicate in &edge.right_property_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, registry, entries)?;
            }
        }
        JoinTree::Questioned { child, edge, .. } => {
            collect_subqueries_in_join_tree(child, analyzed, registry, entries)?;
            for predicate in &edge.property_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, registry, entries)?;
            }
            for predicate in &edge.right_property_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, registry, entries)?;
            }
        }
        JoinTree::Repeat { child, edge, .. } => {
            collect_subqueries_in_join_tree(child, analyzed, registry, entries)?;
            for predicate in &edge.property_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, registry, entries)?;
            }
            for predicate in &edge.inline_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, registry, entries)?;
            }
            for predicate in &edge.final_property_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, registry, entries)?;
            }
        }
        JoinTree::PathSearch { child, .. } | JoinTree::PathModeFilter { child, .. } => {
            collect_subqueries_in_join_tree(child, analyzed, registry, entries)?;
        }
        JoinTree::HashJoin { left, right, .. } => {
            collect_subqueries_in_join_tree(left, analyzed, registry, entries)?;
            collect_subqueries_in_join_tree(right, analyzed, registry, entries)?;
        }
        JoinTree::Outer {
            left,
            right,
            right_filters,
            ..
        } => {
            collect_subqueries_in_join_tree(left, analyzed, registry, entries)?;
            collect_subqueries_in_join_tree(right, analyzed, registry, entries)?;
            for predicate in right_filters {
                collect_subqueries_in_expr(&predicate.expr, analyzed, registry, entries)?;
            }
        }
        JoinTree::WorstCaseOptimal { intersection, .. } => {
            for branch in intersection {
                collect_subqueries_in_join_tree(branch, analyzed, registry, entries)?;
            }
        }
        JoinTree::Subplan(plan) => {
            populate_plan_subqueries(plan, analyzed, registry)?;
        }
    }
    Ok(())
}

fn collect_subqueries_in_mutation(
    op: &MutationOp,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    match op {
        MutationOp::InsertNode { property_inits, .. }
        | MutationOp::InsertEdge { property_inits, .. } => {
            for init in property_inits {
                collect_subqueries_in_project(&init.value, analyzed, registry, entries)?;
            }
        }
        MutationOp::SetProperty { value, .. } => {
            collect_subqueries_in_project(value, analyzed, registry, entries)?;
        }
        MutationOp::SetLabel { .. }
        | MutationOp::RemoveProperty { .. }
        | MutationOp::RemoveLabel { .. }
        | MutationOp::DeleteTarget { .. } => {}
    }
    Ok(())
}

fn collect_subqueries_in_catalog(
    op: &CatalogOp,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    match op {
        CatalogOp::CreateNodeType { properties, .. }
        | CatalogOp::CreateEdgeType { properties, .. } => {
            for property in properties {
                for constraint in &property.constraints {
                    if let PlannedTypePropertyConstraint::Default(project, _) = constraint {
                        collect_subqueries_in_project(project, analyzed, registry, entries)?;
                    }
                }
            }
        }
        CatalogOp::CreateGraph { .. }
        | CatalogOp::DropGraph { .. }
        | CatalogOp::DropNodeType { .. }
        | CatalogOp::DropEdgeType { .. }
        | CatalogOp::ShowNodeTypes(_)
        | CatalogOp::ShowEdgeTypes(_)
        | CatalogOp::ShowIndexes(_)
        | CatalogOp::ShowProcedures(_) => {}
    }
    Ok(())
}

fn collect_subqueries_in_project(
    project: &ProjectExpr,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    collect_subqueries_in_expr(&project.expr, analyzed, registry, entries)
}

fn collect_subqueries_in_expr(
    expr: &ValueExpr,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    match expr {
        ValueExpr::Literal(_) | ValueExpr::Variable { .. } | ValueExpr::Parameter { .. } => {}
        ValueExpr::PropertyAccess { target, .. } => {
            collect_subqueries_in_expr(target, analyzed, registry, entries)?;
        }
        ValueExpr::ListAccess { target, index, .. } => {
            collect_subqueries_in_expr(target, analyzed, registry, entries)?;
            collect_subqueries_in_expr(index, analyzed, registry, entries)?;
        }
        ValueExpr::ListLiteral { items, .. } => {
            for item in items {
                collect_subqueries_in_expr(item, analyzed, registry, entries)?;
            }
        }
        ValueExpr::RecordLiteral { fields, .. } => {
            for (_, value) in fields {
                collect_subqueries_in_expr(value, analyzed, registry, entries)?;
            }
        }
        ValueExpr::BinaryOp { lhs, rhs, .. } => {
            collect_subqueries_in_expr(lhs, analyzed, registry, entries)?;
            collect_subqueries_in_expr(rhs, analyzed, registry, entries)?;
        }
        ValueExpr::UnaryOp { operand, .. } => {
            collect_subqueries_in_expr(operand, analyzed, registry, entries)?;
        }
        ValueExpr::FunctionCall { args, .. } => {
            for arg in args {
                collect_subqueries_in_expr(arg, analyzed, registry, entries)?;
            }
        }
        ValueExpr::IsCheck { operand, kind, .. } => {
            collect_subqueries_in_expr(operand, analyzed, registry, entries)?;
            collect_subqueries_in_is_check(kind, analyzed, registry, entries)?;
        }
        ValueExpr::InList { operand, list, .. } => {
            collect_subqueries_in_expr(operand, analyzed, registry, entries)?;
            for item in list {
                collect_subqueries_in_expr(item, analyzed, registry, entries)?;
            }
        }
        ValueExpr::Like {
            operand, pattern, ..
        } => {
            collect_subqueries_in_expr(operand, analyzed, registry, entries)?;
            collect_subqueries_in_expr(pattern, analyzed, registry, entries)?;
        }
        ValueExpr::Between {
            operand, low, high, ..
        } => {
            collect_subqueries_in_expr(operand, analyzed, registry, entries)?;
            collect_subqueries_in_expr(low, analyzed, registry, entries)?;
            collect_subqueries_in_expr(high, analyzed, registry, entries)?;
        }
        ValueExpr::AllDifferent { items, .. } | ValueExpr::Same { items, .. } => {
            for item in items {
                collect_subqueries_in_expr(item, analyzed, registry, entries)?;
            }
        }
        ValueExpr::PropertyExists { target, .. } => {
            collect_subqueries_in_expr(target, analyzed, registry, entries)?;
        }
        ValueExpr::Case {
            branches,
            else_branch,
            ..
        } => {
            for (condition, value) in branches {
                collect_subqueries_in_expr(condition, analyzed, registry, entries)?;
                collect_subqueries_in_expr(value, analyzed, registry, entries)?;
            }
            if let Some(value) = else_branch {
                collect_subqueries_in_expr(value, analyzed, registry, entries)?;
            }
        }
        ValueExpr::Exists {
            pattern,
            negated,
            span,
        } => {
            collect_planned_subquery(
                expr,
                SubqueryKind::Exists { negated: *negated },
                pattern,
                *span,
                analyzed,
                registry,
                entries,
            )?;
        }
        ValueExpr::CountSubquery { pattern, span } => {
            collect_planned_subquery(
                expr,
                SubqueryKind::Count,
                pattern,
                *span,
                analyzed,
                registry,
                entries,
            )?;
        }
        ValueExpr::ValueSubquery { body, span } => {
            collect_value_subquery(expr, body, *span, analyzed, registry, entries)?;
        }
    }
    Ok(())
}

fn collect_subqueries_in_is_check(
    kind: &IsCheckKind,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    match kind {
        IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) => {
            collect_subqueries_in_expr(value, analyzed, registry, entries)?;
        }
        IsCheckKind::Null
        | IsCheckKind::Directed
        | IsCheckKind::Labeled(_)
        | IsCheckKind::TruthValue(_)
        | IsCheckKind::Typed(_)
        | IsCheckKind::Normalized(_) => {}
    }
    Ok(())
}

fn collect_planned_subquery(
    expr: &ValueExpr,
    kind: SubqueryKind,
    pattern: &crate::MatchClause,
    span: SourceSpan,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    let expr_id = analyzed
        .expr_ids
        .get(expr)
        .ok_or(PlannerError::ExpressionTypeMissing { span })?;
    let mut plan = match_clause::lower_match_prefix(&[pattern], analyzed)?.ok_or(
        PlannerError::NotImplemented {
            feature: "empty graph pattern",
            span: pattern.span,
        },
    )?;
    collect_subqueries_in_pattern_plan(&mut plan, analyzed, registry, entries)?;
    entries.push((
        expr_id,
        PlannedSubquery {
            kind,
            body: SubqueryBody::Pattern(plan),
            outer_binding_refs: outer_binding_refs_in_match(pattern, span, analyzed)?,
            span,
        },
    ));
    Ok(())
}

fn collect_value_subquery(
    expr: &ValueExpr,
    body: &crate::QueryPipeline,
    span: SourceSpan,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    let expr_id = analyzed
        .expr_ids
        .get(expr)
        .ok_or(PlannerError::ExpressionTypeMissing { span })?;
    let mut plan = super::lower_query_pipeline(body, registry, analyzed)?;
    populate_plan_subqueries(&mut plan, analyzed, registry)?;
    entries.push((
        expr_id,
        PlannedSubquery {
            kind: SubqueryKind::Value,
            body: SubqueryBody::Plan(Box::new(plan)),
            outer_binding_refs: outer_binding_refs_in_span(body.span, analyzed)?,
            span,
        },
    ));
    Ok(())
}

fn outer_binding_refs_in_match(
    pattern: &crate::MatchClause,
    subquery_span: SourceSpan,
    analyzed: &AnalyzedStatement,
) -> Result<Vec<OuterBindingRef>, PlannerError> {
    Ok(
        outer_binding_uses_in_match(pattern, subquery_span, analyzed)?
            .into_iter()
            .map(|(binding, name, _)| OuterBindingRef { binding, name })
            .collect(),
    )
}

pub(crate) fn outer_binding_refs_in_span(
    subquery_span: SourceSpan,
    analyzed: &AnalyzedStatement,
) -> Result<Vec<OuterBindingRef>, PlannerError> {
    Ok(
        outer_binding_uses_in_span(subquery_span, subquery_span, analyzed)?
            .into_iter()
            .map(|(binding, name, _)| OuterBindingRef { binding, name })
            .collect(),
    )
}

pub(super) fn outer_binding_uses_in_match(
    pattern: &crate::MatchClause,
    subquery_span: SourceSpan,
    analyzed: &AnalyzedStatement,
) -> Result<Vec<(BindingId, selene_core::IStr, SourceSpan)>, PlannerError> {
    outer_binding_uses_in_span(pattern.span, subquery_span, analyzed)
}

pub(super) fn outer_binding_uses_in_span(
    local_span: SourceSpan,
    subquery_span: SourceSpan,
    analyzed: &AnalyzedStatement,
) -> Result<Vec<(BindingId, selene_core::IStr, SourceSpan)>, PlannerError> {
    let mut refs = Vec::new();
    for reference in &analyzed.references {
        if !span_contains(subquery_span, reference.span) {
            continue;
        }
        let declaration = analyzed.scopes.declaration(reference.binding).ok_or(
            PlannerError::BindingResolutionLost {
                binding: reference.binding,
                span: reference.span,
            },
        )?;
        if !span_contains(local_span, declaration.span()) {
            refs.push((reference.binding, reference.name, reference.span));
        }
    }
    refs.sort_by_key(|(binding, _, _)| *binding);
    refs.dedup_by_key(|(binding, _, _)| *binding);
    Ok(refs)
}

fn span_contains(outer: SourceSpan, inner: SourceSpan) -> bool {
    outer.byte_offset <= inner.byte_offset && inner.end() <= outer.end()
}

/// Aggregate function names recognised by the planner. Mirrors the parser
/// grammar's `aggregate_op` rule (lower-cased after `intern_lower`). A scalar
/// function call with the same arity (e.g. `length(s)`) must not be lifted into
/// `PipelineOp::GroupBy.aggregates`, so this list — not arity — is the gate.
const AGGREGATE_NAMES: &[&str] = &[
    "stddev_samp",
    "stddev_pop",
    "collect_list",
    "collect",
    "count",
    "sum",
    "average",
    "avg",
    "min",
    "max",
];

/// Return aggregate metadata when `expr` is a recognised aggregate call.
///
/// `count(*)` and `count(DISTINCT x)` reach the planner via the parser's
/// `aggregate_expr` rule with `star`/`distinct` set, while bare scalar function
/// calls keep both flags false. Either way, the name must appear in
/// [`AGGREGATE_NAMES`] for the planner to treat it as an aggregate.
pub(crate) fn aggregate_name(expr: &ValueExpr) -> Option<(selene_core::IStr, bool, bool)> {
    let ValueExpr::FunctionCall {
        name,
        star,
        distinct,
        ..
    } = expr
    else {
        return None;
    };
    if name.len() != 1 {
        return None;
    }
    let segment = name[0];
    AGGREGATE_NAMES
        .iter()
        .any(|candidate| segment.as_str() == *candidate)
        .then_some((segment, *star, *distinct))
}
