//! Expression lowering helpers.

use crate::{
    SourceSpan, ValueExpr,
    analyze::{AnalyzedStatement, BindingId, ExprId},
    plan::{
        AggregateArg, FilterPredicate, FilterPredicateKind, OrderKey, PlannerError, ProjectExpr,
    },
};

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
    walk_expr(expr, &mut |sub| {
        if let ValueExpr::Variable { name, span } = sub {
            for reference in analyzed
                .references
                .iter()
                .filter(|reference| reference.name == *name && reference.span == *span)
            {
                refs.push((reference.binding, *span));
            }
        }
    });
    refs.sort_by_key(|(binding, _)| *binding);
    refs.dedup_by_key(|(binding, _)| *binding);
    refs.into_iter()
        .map(|(binding, span)| {
            ensure_binding_exists(binding, span, analyzed)?;
            Ok(binding)
        })
        .collect()
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

fn walk_expr(expr: &ValueExpr, visit: &mut impl FnMut(&ValueExpr)) {
    visit(expr);
    match expr {
        ValueExpr::Literal(_) | ValueExpr::Variable { .. } | ValueExpr::Parameter { .. } => {}
        ValueExpr::PropertyAccess { target, .. } => walk_expr(target, visit),
        ValueExpr::ListAccess { target, index, .. } => {
            walk_expr(target, visit);
            walk_expr(index, visit);
        }
        ValueExpr::ListLiteral { items, .. } => {
            for item in items {
                walk_expr(item, visit);
            }
        }
        ValueExpr::RecordLiteral { fields, .. } => {
            for (_, value) in fields {
                walk_expr(value, visit);
            }
        }
        ValueExpr::BinaryOp { lhs, rhs, .. } => {
            walk_expr(lhs, visit);
            walk_expr(rhs, visit);
        }
        ValueExpr::UnaryOp { operand, .. } => walk_expr(operand, visit),
        ValueExpr::FunctionCall { args, .. } => {
            for arg in args {
                walk_expr(arg, visit);
            }
        }
        ValueExpr::IsCheck { operand, kind, .. } => {
            walk_expr(operand, visit);
            walk_is_check(kind, visit);
        }
        ValueExpr::InList { operand, list, .. } => {
            walk_expr(operand, visit);
            for item in list {
                walk_expr(item, visit);
            }
        }
        ValueExpr::Like {
            operand, pattern, ..
        } => {
            walk_expr(operand, visit);
            walk_expr(pattern, visit);
        }
        ValueExpr::Between {
            operand, low, high, ..
        } => {
            walk_expr(operand, visit);
            walk_expr(low, visit);
            walk_expr(high, visit);
        }
        ValueExpr::AllDifferent { items, .. } | ValueExpr::Same { items, .. } => {
            for item in items {
                walk_expr(item, visit);
            }
        }
        ValueExpr::PropertyExists { target, .. } => walk_expr(target, visit),
        ValueExpr::Case {
            branches,
            else_branch,
            ..
        } => {
            for (condition, value) in branches {
                walk_expr(condition, visit);
                walk_expr(value, visit);
            }
            if let Some(value) = else_branch {
                walk_expr(value, visit);
            }
        }
        ValueExpr::Exists { pattern, .. } | ValueExpr::CountSubquery { pattern, .. } => {
            for graph_pattern in &pattern.patterns {
                for element in &graph_pattern.elements {
                    match element {
                        crate::PatternElement::Node(node) => {
                            for (_, value) in &node.properties {
                                walk_expr(value, visit);
                            }
                            if let Some(value) = &node.inline_where {
                                walk_expr(value, visit);
                            }
                        }
                        crate::PatternElement::Edge(edge) => {
                            for (_, value) in &edge.properties {
                                walk_expr(value, visit);
                            }
                            if let Some(value) = &edge.inline_where {
                                walk_expr(value, visit);
                            }
                        }
                    }
                }
            }
            if let Some(value) = &pattern.where_clause {
                walk_expr(value, visit);
            }
        }
    }
}

fn walk_is_check(kind: &crate::IsCheckKind, visit: &mut impl FnMut(&ValueExpr)) {
    match kind {
        crate::IsCheckKind::SourceOf(value) | crate::IsCheckKind::DestinationOf(value) => {
            walk_expr(value, visit);
        }
        crate::IsCheckKind::Null
        | crate::IsCheckKind::Directed
        | crate::IsCheckKind::Labeled(_)
        | crate::IsCheckKind::TruthValue(_)
        | crate::IsCheckKind::Typed(_)
        | crate::IsCheckKind::Normalized(_) => {}
    }
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
