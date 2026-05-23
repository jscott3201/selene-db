//! Expression lowering helpers.

use crate::{
    IsCheckKind, SourceSpan, ValueExpr,
    analyze::{AnalyzedStatement, BindingId, ExprId},
    plan::{
        AggregateArg, CatalogOp, ExecutionPlan, FilterPredicate, FilterPredicateKind, JoinTree,
        MutationOp, OrderKey, OuterBindingRef, PipelineOp, PlannedSubquery,
        PlannedTypePropertyConstraint, PlannerError, ProjectExpr, SubqueryKind,
    },
};

use super::match_clause;

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
) -> Result<(), PlannerError> {
    let mut entries = Vec::new();
    if let Some(pattern) = plan.pattern_plan.as_mut() {
        collect_subqueries_in_pattern_plan(pattern, analyzed, &mut entries)?;
    }
    for op in &mut plan.pipeline {
        collect_subqueries_in_pipeline_op(op, analyzed, &mut entries)?;
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

/// Bindings referenced by `expr`, sorted and deduplicated.
pub(crate) fn binding_refs_in(
    expr: &ValueExpr,
    analyzed: &AnalyzedStatement,
) -> Result<Vec<BindingId>, PlannerError> {
    let mut refs = Vec::new();
    collect_binding_refs_in_expr(expr, analyzed, &mut refs)?;
    refs.sort_by_key(|(binding, _)| *binding);
    refs.dedup_by_key(|(binding, _)| *binding);
    refs.into_iter()
        .map(|(binding, span)| {
            ensure_binding_exists(binding, span, analyzed)?;
            Ok(binding)
        })
        .collect()
}

fn collect_binding_refs_in_expr(
    expr: &ValueExpr,
    analyzed: &AnalyzedStatement,
    refs: &mut Vec<(BindingId, SourceSpan)>,
) -> Result<(), PlannerError> {
    match expr {
        ValueExpr::Literal(_) | ValueExpr::Parameter { .. } => {}
        ValueExpr::Variable { name, span } => {
            refs.extend(
                analyzed
                    .references
                    .iter()
                    .filter(|reference| reference.name == *name && reference.span == *span)
                    .map(|reference| (reference.binding, *span)),
            );
        }
        ValueExpr::PropertyAccess { target, .. } => {
            collect_binding_refs_in_expr(target, analyzed, refs)?;
        }
        ValueExpr::ListAccess { target, index, .. } => {
            collect_binding_refs_in_expr(target, analyzed, refs)?;
            collect_binding_refs_in_expr(index, analyzed, refs)?;
        }
        ValueExpr::ListLiteral { items, .. } => {
            for item in items {
                collect_binding_refs_in_expr(item, analyzed, refs)?;
            }
        }
        ValueExpr::RecordLiteral { fields, .. } => {
            for (_, value) in fields {
                collect_binding_refs_in_expr(value, analyzed, refs)?;
            }
        }
        ValueExpr::BinaryOp { lhs, rhs, .. } => {
            collect_binding_refs_in_expr(lhs, analyzed, refs)?;
            collect_binding_refs_in_expr(rhs, analyzed, refs)?;
        }
        ValueExpr::UnaryOp { operand, .. } => {
            collect_binding_refs_in_expr(operand, analyzed, refs)?;
        }
        ValueExpr::FunctionCall { args, .. } => {
            for arg in args {
                collect_binding_refs_in_expr(arg, analyzed, refs)?;
            }
        }
        ValueExpr::IsCheck { operand, kind, .. } => {
            collect_binding_refs_in_expr(operand, analyzed, refs)?;
            collect_binding_refs_in_is_check(kind, analyzed, refs)?;
        }
        ValueExpr::InList { operand, list, .. } => {
            collect_binding_refs_in_expr(operand, analyzed, refs)?;
            for item in list {
                collect_binding_refs_in_expr(item, analyzed, refs)?;
            }
        }
        ValueExpr::Like {
            operand, pattern, ..
        } => {
            collect_binding_refs_in_expr(operand, analyzed, refs)?;
            collect_binding_refs_in_expr(pattern, analyzed, refs)?;
        }
        ValueExpr::Between {
            operand, low, high, ..
        } => {
            collect_binding_refs_in_expr(operand, analyzed, refs)?;
            collect_binding_refs_in_expr(low, analyzed, refs)?;
            collect_binding_refs_in_expr(high, analyzed, refs)?;
        }
        ValueExpr::AllDifferent { items, .. } | ValueExpr::Same { items, .. } => {
            for item in items {
                collect_binding_refs_in_expr(item, analyzed, refs)?;
            }
        }
        ValueExpr::PropertyExists { target, .. } => {
            collect_binding_refs_in_expr(target, analyzed, refs)?;
        }
        ValueExpr::Case {
            branches,
            else_branch,
            ..
        } => {
            for (condition, value) in branches {
                collect_binding_refs_in_expr(condition, analyzed, refs)?;
                collect_binding_refs_in_expr(value, analyzed, refs)?;
            }
            if let Some(value) = else_branch {
                collect_binding_refs_in_expr(value, analyzed, refs)?;
            }
        }
        ValueExpr::Exists { pattern, span, .. } | ValueExpr::CountSubquery { pattern, span } => {
            refs.extend(
                outer_binding_uses_in_match(pattern, *span, analyzed)?
                    .into_iter()
                    .map(|(binding, _, span)| (binding, span)),
            );
        }
        ValueExpr::ValueSubquery { .. } => {}
    }
    Ok(())
}

fn collect_binding_refs_in_is_check(
    kind: &IsCheckKind,
    analyzed: &AnalyzedStatement,
    refs: &mut Vec<(BindingId, SourceSpan)>,
) -> Result<(), PlannerError> {
    match kind {
        IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) => {
            collect_binding_refs_in_expr(value, analyzed, refs)?;
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
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    match op {
        PipelineOp::Filter(predicate) => {
            collect_subqueries_in_expr(&predicate.expr, analyzed, entries)?;
        }
        PipelineOp::Project(projects) | PipelineOp::Let(projects) => {
            for project in projects {
                collect_subqueries_in_project(project, analyzed, entries)?;
            }
        }
        PipelineOp::Unwind { source, .. } => {
            collect_subqueries_in_project(source, analyzed, entries)?;
        }
        PipelineOp::OrderBy(keys) | PipelineOp::TopK { keys, .. } => {
            for key in keys {
                collect_subqueries_in_expr(&key.expr, analyzed, entries)?;
            }
        }
        PipelineOp::GroupBy { keys, aggregates } => {
            for key in keys {
                collect_subqueries_in_project(key, analyzed, entries)?;
            }
            for aggregate in aggregates {
                for arg in &aggregate.args {
                    collect_subqueries_in_expr(&arg.expr, analyzed, entries)?;
                }
            }
        }
        PipelineOp::Union { rhs, .. } | PipelineOp::Chain(rhs) => {
            populate_plan_subqueries(rhs, analyzed)?;
        }
        PipelineOp::Match(pattern) => {
            collect_subqueries_in_pattern_plan(pattern, analyzed, entries)?
        }
        PipelineOp::ExplainPlan { inner, .. } => populate_plan_subqueries(inner, analyzed)?,
        PipelineOp::Call(call) => {
            for arg in &call.args {
                collect_subqueries_in_project(arg, analyzed, entries)?;
            }
        }
        PipelineOp::Mutation(op) => collect_subqueries_in_mutation(op, analyzed, entries)?,
        PipelineOp::Catalog(op) => collect_subqueries_in_catalog(op, analyzed, entries)?,
        PipelineOp::Limit { .. } | PipelineOp::Distinct | PipelineOp::Tx(_) => {}
    }
    Ok(())
}

fn collect_subqueries_in_pattern_plan(
    pattern: &mut crate::PatternPlan,
    analyzed: &AnalyzedStatement,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    for filter in &pattern.filters {
        collect_subqueries_in_expr(&filter.expr, analyzed, entries)?;
    }
    collect_subqueries_in_join_tree(&mut pattern.join_tree, analyzed, entries)
}

fn collect_subqueries_in_join_tree(
    tree: &mut JoinTree,
    analyzed: &AnalyzedStatement,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    match tree {
        JoinTree::Scan(scan) => {
            for predicate in &scan.property_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, entries)?;
            }
        }
        JoinTree::Expand { child, edge, .. } => {
            collect_subqueries_in_join_tree(child, analyzed, entries)?;
            for predicate in &edge.property_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, entries)?;
            }
            for predicate in &edge.right_property_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, entries)?;
            }
        }
        JoinTree::Questioned { child, edge, .. } => {
            collect_subqueries_in_join_tree(child, analyzed, entries)?;
            for predicate in &edge.property_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, entries)?;
            }
            for predicate in &edge.right_property_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, entries)?;
            }
        }
        JoinTree::Repeat { child, edge, .. } => {
            collect_subqueries_in_join_tree(child, analyzed, entries)?;
            for predicate in &edge.property_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, entries)?;
            }
            for predicate in &edge.inline_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, entries)?;
            }
            for predicate in &edge.final_property_predicates {
                collect_subqueries_in_expr(&predicate.expr, analyzed, entries)?;
            }
        }
        JoinTree::PathSearch { child, .. } | JoinTree::PathModeFilter { child, .. } => {
            collect_subqueries_in_join_tree(child, analyzed, entries)?;
        }
        JoinTree::HashJoin { left, right, .. } => {
            collect_subqueries_in_join_tree(left, analyzed, entries)?;
            collect_subqueries_in_join_tree(right, analyzed, entries)?;
        }
        JoinTree::Outer {
            left,
            right,
            right_filters,
            ..
        } => {
            collect_subqueries_in_join_tree(left, analyzed, entries)?;
            collect_subqueries_in_join_tree(right, analyzed, entries)?;
            for predicate in right_filters {
                collect_subqueries_in_expr(&predicate.expr, analyzed, entries)?;
            }
        }
        JoinTree::WorstCaseOptimal { intersection, .. } => {
            for branch in intersection {
                collect_subqueries_in_join_tree(branch, analyzed, entries)?;
            }
        }
        JoinTree::Subplan(plan) => {
            populate_plan_subqueries(plan, analyzed)?;
        }
    }
    Ok(())
}

fn collect_subqueries_in_mutation(
    op: &MutationOp,
    analyzed: &AnalyzedStatement,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    match op {
        MutationOp::InsertNode { property_inits, .. }
        | MutationOp::InsertEdge { property_inits, .. } => {
            for init in property_inits {
                collect_subqueries_in_project(&init.value, analyzed, entries)?;
            }
        }
        MutationOp::SetProperty { value, .. } => {
            collect_subqueries_in_project(value, analyzed, entries)?;
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
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    match op {
        CatalogOp::CreateNodeType { properties, .. }
        | CatalogOp::CreateEdgeType { properties, .. } => {
            for property in properties {
                for constraint in &property.constraints {
                    if let PlannedTypePropertyConstraint::Default(project, _) = constraint {
                        collect_subqueries_in_project(project, analyzed, entries)?;
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
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    collect_subqueries_in_expr(&project.expr, analyzed, entries)
}

fn collect_subqueries_in_expr(
    expr: &ValueExpr,
    analyzed: &AnalyzedStatement,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    match expr {
        ValueExpr::Literal(_) | ValueExpr::Variable { .. } | ValueExpr::Parameter { .. } => {}
        ValueExpr::PropertyAccess { target, .. } => {
            collect_subqueries_in_expr(target, analyzed, entries)?;
        }
        ValueExpr::ListAccess { target, index, .. } => {
            collect_subqueries_in_expr(target, analyzed, entries)?;
            collect_subqueries_in_expr(index, analyzed, entries)?;
        }
        ValueExpr::ListLiteral { items, .. } => {
            for item in items {
                collect_subqueries_in_expr(item, analyzed, entries)?;
            }
        }
        ValueExpr::RecordLiteral { fields, .. } => {
            for (_, value) in fields {
                collect_subqueries_in_expr(value, analyzed, entries)?;
            }
        }
        ValueExpr::BinaryOp { lhs, rhs, .. } => {
            collect_subqueries_in_expr(lhs, analyzed, entries)?;
            collect_subqueries_in_expr(rhs, analyzed, entries)?;
        }
        ValueExpr::UnaryOp { operand, .. } => {
            collect_subqueries_in_expr(operand, analyzed, entries)?;
        }
        ValueExpr::FunctionCall { args, .. } => {
            for arg in args {
                collect_subqueries_in_expr(arg, analyzed, entries)?;
            }
        }
        ValueExpr::IsCheck { operand, kind, .. } => {
            collect_subqueries_in_expr(operand, analyzed, entries)?;
            collect_subqueries_in_is_check(kind, analyzed, entries)?;
        }
        ValueExpr::InList { operand, list, .. } => {
            collect_subqueries_in_expr(operand, analyzed, entries)?;
            for item in list {
                collect_subqueries_in_expr(item, analyzed, entries)?;
            }
        }
        ValueExpr::Like {
            operand, pattern, ..
        } => {
            collect_subqueries_in_expr(operand, analyzed, entries)?;
            collect_subqueries_in_expr(pattern, analyzed, entries)?;
        }
        ValueExpr::Between {
            operand, low, high, ..
        } => {
            collect_subqueries_in_expr(operand, analyzed, entries)?;
            collect_subqueries_in_expr(low, analyzed, entries)?;
            collect_subqueries_in_expr(high, analyzed, entries)?;
        }
        ValueExpr::AllDifferent { items, .. } | ValueExpr::Same { items, .. } => {
            for item in items {
                collect_subqueries_in_expr(item, analyzed, entries)?;
            }
        }
        ValueExpr::PropertyExists { target, .. } => {
            collect_subqueries_in_expr(target, analyzed, entries)?;
        }
        ValueExpr::Case {
            branches,
            else_branch,
            ..
        } => {
            for (condition, value) in branches {
                collect_subqueries_in_expr(condition, analyzed, entries)?;
                collect_subqueries_in_expr(value, analyzed, entries)?;
            }
            if let Some(value) = else_branch {
                collect_subqueries_in_expr(value, analyzed, entries)?;
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
                entries,
            )?;
        }
        ValueExpr::CountSubquery { pattern, span } => {
            collect_planned_subquery(expr, SubqueryKind::Count, pattern, *span, analyzed, entries)?;
        }
        ValueExpr::ValueSubquery { .. } => {}
    }
    Ok(())
}

fn collect_subqueries_in_is_check(
    kind: &IsCheckKind,
    analyzed: &AnalyzedStatement,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
) -> Result<(), PlannerError> {
    match kind {
        IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) => {
            collect_subqueries_in_expr(value, analyzed, entries)?;
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
    collect_subqueries_in_pattern_plan(&mut plan, analyzed, entries)?;
    entries.push((
        expr_id,
        PlannedSubquery {
            kind,
            plan,
            outer_binding_refs: outer_binding_refs_in_match(pattern, span, analyzed)?,
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

fn outer_binding_uses_in_match(
    pattern: &crate::MatchClause,
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
        if !span_contains(pattern.span, declaration.span()) {
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
