//! Expression lowering helpers.

use crate::{
    ExistsBody, ProcedureRegistry, SourceSpan, ValueExpr,
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
    alias: Option<selene_core::DbString>,
    analyzed: &AnalyzedStatement,
) -> Result<ProjectExpr, PlannerError> {
    let (expr_id, ty) = expr_cell(expr, analyzed)?;
    Ok(ProjectExpr {
        expr: expr.clone(),
        expr_id,
        ty,
        declared_type: None,
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
    key: selene_core::DbString,
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
///
/// `max_quantifier` is threaded so a variable-length quantifier *inside* a
/// subquery body is gated against the embedder's [`ImplDefinedCaps`] cap, the
/// same as a top-level quantifier (the gate fires when the subquery body is
/// lowered via [`super::lower_query_pipeline`] / `lower_match_prefix`).
pub(crate) fn populate_plan_subqueries(
    plan: &mut ExecutionPlan,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    max_quantifier: u32,
) -> Result<(), PlannerError> {
    let mut entries = Vec::new();
    if let Some(pattern) = plan.pattern_plan.as_mut() {
        collect_subqueries_in_pattern_plan(
            pattern,
            analyzed,
            registry,
            &mut entries,
            max_quantifier,
        )?;
    }
    for op in &mut plan.pipeline {
        collect_subqueries_in_pipeline_op(op, analyzed, registry, &mut entries, max_quantifier)?;
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
    max_quantifier: u32,
) -> Result<(), PlannerError> {
    match op {
        PipelineOp::Filter(predicate) => {
            collect_subqueries_in_expr(
                &predicate.expr,
                analyzed,
                registry,
                entries,
                max_quantifier,
            )?;
        }
        PipelineOp::Project(projects) | PipelineOp::Let(projects) => {
            for project in projects {
                collect_subqueries_in_project(
                    project,
                    analyzed,
                    registry,
                    entries,
                    max_quantifier,
                )?;
            }
        }
        PipelineOp::Unwind { source, .. } => {
            collect_subqueries_in_project(source, analyzed, registry, entries, max_quantifier)?;
        }
        PipelineOp::OrderBy(keys) | PipelineOp::TopK { keys, .. } => {
            for key in keys {
                collect_subqueries_in_expr(&key.expr, analyzed, registry, entries, max_quantifier)?;
            }
        }
        PipelineOp::GroupBy { keys, aggregates } => {
            for key in keys {
                collect_subqueries_in_project(key, analyzed, registry, entries, max_quantifier)?;
            }
            for aggregate in aggregates {
                for arg in &aggregate.args {
                    collect_subqueries_in_expr(
                        &arg.expr,
                        analyzed,
                        registry,
                        entries,
                        max_quantifier,
                    )?;
                }
            }
        }
        PipelineOp::Union { rhs, .. }
        | PipelineOp::Chain(rhs)
        | PipelineOp::CorrelatedChain(rhs) => {
            populate_plan_subqueries(rhs, analyzed, registry, max_quantifier)?;
        }
        PipelineOp::Match(pattern) | PipelineOp::OptionalMatch(pattern) => {
            collect_subqueries_in_pattern_plan(
                pattern,
                analyzed,
                registry,
                entries,
                max_quantifier,
            )?
        }
        PipelineOp::ExplainPlan { inner, .. } => {
            populate_plan_subqueries(inner, analyzed, registry, max_quantifier)?
        }
        PipelineOp::Call(call) => {
            for arg in &call.args {
                collect_subqueries_in_project(arg, analyzed, registry, entries, max_quantifier)?;
            }
        }
        PipelineOp::CallSubquery(call) => {
            populate_plan_subqueries(&mut call.body, analyzed, registry, max_quantifier)?;
        }
        PipelineOp::Mutation(op) => {
            collect_subqueries_in_mutation(op, analyzed, registry, entries, max_quantifier)?
        }
        PipelineOp::Catalog(op) => {
            collect_subqueries_in_catalog(op, analyzed, registry, entries, max_quantifier)?
        }
        PipelineOp::Limit { .. }
        | PipelineOp::Distinct
        | PipelineOp::Tx(_)
        | PipelineOp::Session(_) => {}
    }
    Ok(())
}

fn collect_subqueries_in_pattern_plan(
    pattern: &mut crate::PatternPlan,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
    max_quantifier: u32,
) -> Result<(), PlannerError> {
    for filter in &pattern.filters {
        collect_subqueries_in_expr(&filter.expr, analyzed, registry, entries, max_quantifier)?;
    }
    collect_subqueries_in_join_tree(
        &mut pattern.join_tree,
        analyzed,
        registry,
        entries,
        max_quantifier,
    )
}

fn collect_subqueries_in_join_tree(
    tree: &mut JoinTree,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
    max_quantifier: u32,
) -> Result<(), PlannerError> {
    match tree {
        JoinTree::Unit => {}
        JoinTree::Scan(scan) => {
            for predicate in &scan.property_predicates {
                collect_subqueries_in_expr(
                    &predicate.expr,
                    analyzed,
                    registry,
                    entries,
                    max_quantifier,
                )?;
            }
        }
        JoinTree::Expand { child, edge, .. } => {
            collect_subqueries_in_join_tree(child, analyzed, registry, entries, max_quantifier)?;
            for predicate in &edge.property_predicates {
                collect_subqueries_in_expr(
                    &predicate.expr,
                    analyzed,
                    registry,
                    entries,
                    max_quantifier,
                )?;
            }
            for predicate in &edge.right_property_predicates {
                collect_subqueries_in_expr(
                    &predicate.expr,
                    analyzed,
                    registry,
                    entries,
                    max_quantifier,
                )?;
            }
        }
        JoinTree::Questioned { child, edge, .. } => {
            collect_subqueries_in_join_tree(child, analyzed, registry, entries, max_quantifier)?;
            for predicate in &edge.property_predicates {
                collect_subqueries_in_expr(
                    &predicate.expr,
                    analyzed,
                    registry,
                    entries,
                    max_quantifier,
                )?;
            }
            for predicate in &edge.right_property_predicates {
                collect_subqueries_in_expr(
                    &predicate.expr,
                    analyzed,
                    registry,
                    entries,
                    max_quantifier,
                )?;
            }
        }
        JoinTree::Repeat { child, edge, .. } => {
            collect_subqueries_in_join_tree(child, analyzed, registry, entries, max_quantifier)?;
            for predicate in &edge.property_predicates {
                collect_subqueries_in_expr(
                    &predicate.expr,
                    analyzed,
                    registry,
                    entries,
                    max_quantifier,
                )?;
            }
            for predicate in &edge.inline_predicates {
                collect_subqueries_in_expr(
                    &predicate.expr,
                    analyzed,
                    registry,
                    entries,
                    max_quantifier,
                )?;
            }
            for predicate in &edge.final_property_predicates {
                collect_subqueries_in_expr(
                    &predicate.expr,
                    analyzed,
                    registry,
                    entries,
                    max_quantifier,
                )?;
            }
        }
        JoinTree::PathSearch { child, .. }
        | JoinTree::PathModeFilter { child, .. }
        | JoinTree::MatchModeFilter { child, .. } => {
            collect_subqueries_in_join_tree(child, analyzed, registry, entries, max_quantifier)?;
        }
        JoinTree::HashJoin { left, right, .. } => {
            collect_subqueries_in_join_tree(left, analyzed, registry, entries, max_quantifier)?;
            collect_subqueries_in_join_tree(right, analyzed, registry, entries, max_quantifier)?;
        }
        JoinTree::Outer {
            left,
            right,
            right_filters,
            ..
        } => {
            collect_subqueries_in_join_tree(left, analyzed, registry, entries, max_quantifier)?;
            collect_subqueries_in_join_tree(right, analyzed, registry, entries, max_quantifier)?;
            for predicate in right_filters {
                collect_subqueries_in_expr(
                    &predicate.expr,
                    analyzed,
                    registry,
                    entries,
                    max_quantifier,
                )?;
            }
        }
        JoinTree::WorstCaseOptimal { intersection, .. } => {
            for branch in intersection {
                collect_subqueries_in_join_tree(
                    branch,
                    analyzed,
                    registry,
                    entries,
                    max_quantifier,
                )?;
            }
        }
        JoinTree::Subplan(plan) => {
            populate_plan_subqueries(plan, analyzed, registry, max_quantifier)?;
        }
        JoinTree::DisjunctiveScan { .. } => {
            // DisjunctiveScan is emitted by the disjunctive_label_expansion
            // optimizer rule (post-lowering); subquery collection runs over
            // freshly-lowered plans only.
            unreachable!("DisjunctiveScan is rule-emitted post-lowering")
        }
    }
    Ok(())
}

fn collect_subqueries_in_mutation(
    op: &MutationOp,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
    max_quantifier: u32,
) -> Result<(), PlannerError> {
    match op {
        MutationOp::InsertNode { property_inits, .. }
        | MutationOp::InsertEdge { property_inits, .. } => {
            for init in property_inits {
                collect_subqueries_in_project(
                    &init.value,
                    analyzed,
                    registry,
                    entries,
                    max_quantifier,
                )?;
            }
        }
        MutationOp::SetProperty { value, .. } => {
            collect_subqueries_in_project(value, analyzed, registry, entries, max_quantifier)?;
        }
        MutationOp::SetLabel { .. }
        | MutationOp::RemoveProperty { .. }
        | MutationOp::RemoveLabel { .. }
        | MutationOp::DeleteTargets { .. } => {}
    }
    Ok(())
}

fn collect_subqueries_in_catalog(
    op: &CatalogOp,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
    max_quantifier: u32,
) -> Result<(), PlannerError> {
    match op {
        CatalogOp::CreateNodeType { properties, .. }
        | CatalogOp::CreateEdgeType { properties, .. } => {
            for property in properties {
                for constraint in &property.constraints {
                    if let PlannedTypePropertyConstraint::Default(project, _) = constraint {
                        collect_subqueries_in_project(
                            project,
                            analyzed,
                            registry,
                            entries,
                            max_quantifier,
                        )?;
                    }
                }
            }
        }
        CatalogOp::CreateGraph { .. }
        | CatalogOp::DropGraph { .. }
        | CatalogOp::DropNodeType { .. }
        | CatalogOp::DropEdgeType { .. }
        | CatalogOp::TruncateNodeType { .. }
        | CatalogOp::TruncateEdgeType { .. }
        | CatalogOp::CreateIndex { .. }
        | CatalogOp::DropIndex { .. }
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
    max_quantifier: u32,
) -> Result<(), PlannerError> {
    collect_subqueries_in_expr(&project.expr, analyzed, registry, entries, max_quantifier)
}

fn collect_subqueries_in_expr(
    expr: &ValueExpr,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
    max_quantifier: u32,
) -> Result<(), PlannerError> {
    match expr {
        // Subquery bodies are `MatchClause` / `QueryPipeline`, not `ValueExpr`
        // children: register the planned subquery (which recurses into its own
        // body) rather than walking through `for_each_child`.
        ValueExpr::Exists {
            body,
            negated,
            span,
        } => match body {
            ExistsBody::Match(pattern) => {
                collect_planned_match_subquery(
                    expr,
                    SubqueryKind::Exists { negated: *negated },
                    pattern,
                    *span,
                    analyzed,
                    registry,
                    entries,
                    max_quantifier,
                )?;
            }
            ExistsBody::Query(body) => {
                collect_planned_query_subquery(
                    expr,
                    SubqueryKind::Exists { negated: *negated },
                    body,
                    *span,
                    analyzed,
                    registry,
                    entries,
                    max_quantifier,
                )?;
            }
        },
        ValueExpr::ValueSubquery { body, span } => {
            collect_planned_query_subquery(
                expr,
                SubqueryKind::Value,
                body,
                *span,
                analyzed,
                registry,
                entries,
                max_quantifier,
            )?;
        }
        // Every other variant only recurses into its direct `ValueExpr`
        // children (including the `IS [SOURCE|DESTINATION] OF` operand).
        _ => {
            let mut result = Ok(());
            expr.for_each_child(&mut |child| {
                if result.is_ok() {
                    result = collect_subqueries_in_expr(
                        child,
                        analyzed,
                        registry,
                        entries,
                        max_quantifier,
                    );
                }
            });
            result?;
        }
    }
    Ok(())
}

// Why: threads the four shared collection params (analyzed/registry/entries/
// max_quantifier) plus the four subquery descriptors (expr/kind/pattern/span);
// bundling them would not improve clarity for one internal helper.
#[allow(clippy::too_many_arguments)]
fn collect_planned_match_subquery(
    expr: &ValueExpr,
    kind: SubqueryKind,
    pattern: &crate::MatchClause,
    span: SourceSpan,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
    max_quantifier: u32,
) -> Result<(), PlannerError> {
    let expr_id = analyzed
        .expr_ids
        .get(expr)
        .ok_or(PlannerError::ExpressionTypeMissing { span })?;
    let mut plan = match_clause::lower_match_prefix(&[pattern], analyzed, max_quantifier)?.ok_or(
        PlannerError::NotImplemented {
            feature: "empty graph pattern",
            span: pattern.span,
        },
    )?;
    collect_subqueries_in_pattern_plan(&mut plan, analyzed, registry, entries, max_quantifier)?;
    entries.push((
        expr_id,
        PlannedSubquery {
            kind,
            body: SubqueryBody::Pattern(Box::new(plan)),
            outer_binding_refs: outer_binding_refs_in_match(pattern, span, analyzed)?,
            span,
        },
    ));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn collect_planned_query_subquery(
    expr: &ValueExpr,
    kind: SubqueryKind,
    body: &crate::QueryPipeline,
    span: SourceSpan,
    analyzed: &AnalyzedStatement,
    registry: &dyn ProcedureRegistry,
    entries: &mut Vec<(ExprId, PlannedSubquery)>,
    max_quantifier: u32,
) -> Result<(), PlannerError> {
    let expr_id = analyzed
        .expr_ids
        .get(expr)
        .ok_or(PlannerError::ExpressionTypeMissing { span })?;
    let mut plan = super::lower_query_pipeline(body, registry, analyzed, max_quantifier)?;
    populate_plan_subqueries(&mut plan, analyzed, registry, max_quantifier)?;
    entries.push((
        expr_id,
        PlannedSubquery {
            kind,
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
) -> Result<Vec<(BindingId, selene_core::DbString, SourceSpan)>, PlannerError> {
    outer_binding_uses_in_span(pattern.span, subquery_span, analyzed)
}

pub(super) fn outer_binding_uses_in_span(
    local_span: SourceSpan,
    subquery_span: SourceSpan,
    analyzed: &AnalyzedStatement,
) -> Result<Vec<(BindingId, selene_core::DbString, SourceSpan)>, PlannerError> {
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
            refs.push((reference.binding, reference.name.clone(), reference.span));
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
/// grammar's `aggregate_op` rule (lower-cased after `lowercase_db_string`). A scalar
/// function call with the same arity (e.g. `char_length(s)`) must not be lifted into
/// `PipelineOp::GroupBy.aggregates`, so this list — not arity — is the gate.
const AGGREGATE_NAMES: &[&str] = &[
    "stddev_samp",
    "stddev_pop",
    "collect_list",
    "count",
    "sum",
    "avg",
    "min",
    "max",
    "percentile_cont",
    "percentile_disc",
];

/// Return aggregate metadata when `expr` is a recognised aggregate call.
///
/// `count(*)` and `count(DISTINCT x)` reach the planner via the parser's
/// `aggregate_expr` rule with `star`/`distinct` set, while bare scalar function
/// calls keep both flags false. Either way, the name must appear in
/// [`AGGREGATE_NAMES`] for the planner to treat it as an aggregate.
pub(crate) fn aggregate_name(expr: &ValueExpr) -> Option<(selene_core::DbString, bool, bool)> {
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
    let segment = name[0].clone();
    AGGREGATE_NAMES
        .iter()
        .any(|candidate| segment.as_str() == *candidate)
        .then_some((segment, *star, *distinct))
}
