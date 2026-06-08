//! Aggregate-aware projection lowering.

use std::collections::HashMap;

use selene_core::{DbString, db_string};

use crate::{
    ReturnItem, ValueExpr,
    analyze::{AnalyzedStatement, ExprId},
    plan::{
        Aggregate, FilterPredicate, FilterPredicateKind, PipelineOp, PlannerError, ProjectExpr,
    },
};

use super::expr;

#[derive(Default)]
pub(super) struct AggregateRewrite {
    pub(super) aggregates: Vec<Aggregate>,
    pub(super) names_by_expr_id: HashMap<ExprId, DbString>,
}

/// Emit a `GroupBy` op when grouping is required.
///
/// An explicit `GROUP BY` is honored as-is. When `group_by` is absent but at
/// least one aggregate is present, the planner inserts an implicit grouping
/// over the entire input (empty key list), matching GQL's rule that
/// aggregate-only queries collapse to a single output row.
pub(super) fn push_grouping(
    group_by: &Option<Vec<ValueExpr>>,
    items: &[ReturnItem],
    having: Option<&ValueExpr>,
    analyzed: &AnalyzedStatement,
    ops: &mut Vec<PipelineOp>,
) -> Result<AggregateRewrite, PlannerError> {
    let aggregate_rewrite = aggregates(items, having, analyzed)?;
    if let Some(keys) = group_by {
        ops.push(PipelineOp::GroupBy {
            keys: keys
                .iter()
                .map(|value| expr::project_expr(value, group_key_alias(value, items), analyzed))
                .collect::<Result<Vec<_>, _>>()?,
            aggregates: aggregate_rewrite.aggregates.clone(),
        });
    } else if !aggregate_rewrite.aggregates.is_empty() {
        ops.push(PipelineOp::GroupBy {
            keys: Vec::new(),
            aggregates: aggregate_rewrite.aggregates.clone(),
        });
    }
    Ok(aggregate_rewrite)
}

pub(super) fn project_items(
    items: &[ReturnItem],
    analyzed: &AnalyzedStatement,
    aggregate_names: &HashMap<ExprId, DbString>,
) -> Result<Vec<ProjectExpr>, PlannerError> {
    items
        .iter()
        .map(|item| {
            project_expr(
                &item.expr,
                column_name(&item.expr, item.alias.clone()),
                analyzed,
                aggregate_names,
            )
        })
        .collect()
}

pub(super) fn filter_predicate(
    original: &ValueExpr,
    analyzed: &AnalyzedStatement,
    aggregate_names: &HashMap<ExprId, DbString>,
) -> Result<FilterPredicate, PlannerError> {
    let (expr_id, ty) = expr::expr_cell(original, analyzed)?;
    Ok(FilterPredicate {
        expr: rewrite_aggregate_refs(original, aggregate_names, analyzed),
        expr_id,
        ty,
        binding_refs: expr::binding_refs_in(original, analyzed)?,
        kind: FilterPredicateKind::Expression,
        index_consumed: false,
        span: original.span(),
    })
}

fn aggregates(
    items: &[ReturnItem],
    having: Option<&ValueExpr>,
    analyzed: &AnalyzedStatement,
) -> Result<AggregateRewrite, PlannerError> {
    let mut rewrite = AggregateRewrite::default();
    for item in items {
        collect_aggregates(&item.expr, analyzed, &mut rewrite)?;
    }
    if let Some(having) = having {
        collect_aggregates(having, analyzed, &mut rewrite)?;
    }
    Ok(rewrite)
}

fn collect_aggregates(
    value: &ValueExpr,
    analyzed: &AnalyzedStatement,
    rewrite: &mut AggregateRewrite,
) -> Result<(), PlannerError> {
    if let Some((function, star, distinct)) = expr::aggregate_name(value) {
        let (aggregate_id, ty) = expr::expr_cell(value, analyzed)?;
        if rewrite.names_by_expr_id.contains_key(&aggregate_id) {
            return Ok(());
        }
        let output_name = synthesized_aggregate_name(aggregate_id, value.span())?;
        let args = match value {
            ValueExpr::FunctionCall { args, .. } => args,
            _ => unreachable!("aggregate_name only matches function calls"),
        };
        rewrite
            .names_by_expr_id
            .insert(aggregate_id, output_name.clone());
        rewrite.aggregates.push(Aggregate {
            aggregate_id,
            output_name,
            function,
            args: args
                .iter()
                .map(|arg| expr::aggregate_arg(arg, analyzed))
                .collect::<Result<Vec<_>, _>>()?,
            star,
            distinct,
            ty,
            span: value.span(),
        });
        return Ok(());
    }

    match value {
        ValueExpr::Literal(_) | ValueExpr::Variable { .. } | ValueExpr::Parameter { .. } => {}
        ValueExpr::PropertyAccess { target, .. }
        | ValueExpr::UnaryOp {
            operand: target, ..
        }
        | ValueExpr::PropertyExists { target, .. } => {
            collect_aggregates(target, analyzed, rewrite)?;
        }
        ValueExpr::ListAccess { target, index, .. } => {
            collect_aggregates(target, analyzed, rewrite)?;
            collect_aggregates(index, analyzed, rewrite)?;
        }
        ValueExpr::ListLiteral { items, .. }
        | ValueExpr::AllDifferent { items, .. }
        | ValueExpr::Same { items, .. } => {
            for item in items {
                collect_aggregates(item, analyzed, rewrite)?;
            }
        }
        ValueExpr::RecordLiteral { fields, .. } => {
            for (_, field) in fields {
                collect_aggregates(field, analyzed, rewrite)?;
            }
        }
        ValueExpr::BinaryOp { lhs, rhs, .. } => {
            collect_aggregates(lhs, analyzed, rewrite)?;
            collect_aggregates(rhs, analyzed, rewrite)?;
        }
        ValueExpr::FunctionCall { args, .. } => {
            for arg in args {
                collect_aggregates(arg, analyzed, rewrite)?;
            }
        }
        ValueExpr::DurationBetween { start, end, .. } => {
            collect_aggregates(start, analyzed, rewrite)?;
            collect_aggregates(end, analyzed, rewrite)?;
        }
        ValueExpr::IsCheck { operand, kind, .. } => {
            collect_aggregates(operand, analyzed, rewrite)?;
            collect_is_check_aggregates(kind, analyzed, rewrite)?;
        }
        ValueExpr::InList { operand, list, .. } => {
            collect_aggregates(operand, analyzed, rewrite)?;
            for item in list {
                collect_aggregates(item, analyzed, rewrite)?;
            }
        }
        ValueExpr::InListExpression { operand, list, .. } => {
            collect_aggregates(operand, analyzed, rewrite)?;
            collect_aggregates(list, analyzed, rewrite)?;
        }
        ValueExpr::Case {
            branches,
            else_branch,
            ..
        } => {
            for (condition, result) in branches {
                collect_aggregates(condition, analyzed, rewrite)?;
                collect_aggregates(result, analyzed, rewrite)?;
            }
            if let Some(result) = else_branch {
                collect_aggregates(result, analyzed, rewrite)?;
            }
        }
        ValueExpr::Cast { value, .. } => collect_aggregates(value, analyzed, rewrite)?,
        ValueExpr::Normalize { source, .. } => collect_aggregates(source, analyzed, rewrite)?,
        ValueExpr::Trim {
            character, source, ..
        } => {
            if let Some(character) = character {
                collect_aggregates(character, analyzed, rewrite)?;
            }
            collect_aggregates(source, analyzed, rewrite)?;
        }
        ValueExpr::Exists { .. }
        | ValueExpr::CountSubquery { .. }
        | ValueExpr::ValueSubquery { .. } => {}
    }
    Ok(())
}

fn collect_is_check_aggregates(
    kind: &crate::IsCheckKind,
    analyzed: &AnalyzedStatement,
    rewrite: &mut AggregateRewrite,
) -> Result<(), PlannerError> {
    match kind {
        crate::IsCheckKind::SourceOf(value) | crate::IsCheckKind::DestinationOf(value) => {
            collect_aggregates(value, analyzed, rewrite)
        }
        crate::IsCheckKind::Null
        | crate::IsCheckKind::Directed
        | crate::IsCheckKind::Labeled(_)
        | crate::IsCheckKind::TruthValue(_)
        | crate::IsCheckKind::Typed(_)
        | crate::IsCheckKind::Normalized(_) => Ok(()),
    }
}

fn synthesized_aggregate_name(
    expr_id: ExprId,
    span: crate::SourceSpan,
) -> Result<DbString, PlannerError> {
    let name = format!("agg_{}", expr_id.get());
    db_string(&name).map_err(|_err| PlannerError::StaticStringConstructionFailed {
        detail: "aggregate synthesized column",
        span,
    })
}

fn project_expr(
    original: &ValueExpr,
    alias: Option<DbString>,
    analyzed: &AnalyzedStatement,
    aggregate_names: &HashMap<ExprId, DbString>,
) -> Result<ProjectExpr, PlannerError> {
    let (expr_id, ty) = expr::expr_cell(original, analyzed)?;
    Ok(ProjectExpr {
        expr: rewrite_aggregate_refs(original, aggregate_names, analyzed),
        expr_id,
        ty,
        alias,
        binding_refs: expr::binding_refs_in(original, analyzed)?,
        span: original.span(),
    })
}

fn rewrite_aggregate_refs(
    value: &ValueExpr,
    aggregate_names: &HashMap<ExprId, DbString>,
    analyzed: &AnalyzedStatement,
) -> ValueExpr {
    if let Some(name) = analyzed
        .expr_ids
        .get(value)
        .and_then(|expr_id| aggregate_names.get(&expr_id))
    {
        return ValueExpr::Variable {
            name: name.clone(),
            span: value.span(),
        };
    }

    match value {
        ValueExpr::Literal(_)
        | ValueExpr::Variable { .. }
        | ValueExpr::Parameter { .. }
        | ValueExpr::Exists { .. }
        | ValueExpr::CountSubquery { .. }
        | ValueExpr::ValueSubquery { .. } => value.clone(),
        ValueExpr::PropertyAccess { target, key, span } => ValueExpr::PropertyAccess {
            target: Box::new(rewrite_aggregate_refs(target, aggregate_names, analyzed)),
            key: key.clone(),
            span: *span,
        },
        ValueExpr::ListAccess {
            target,
            index,
            span,
        } => ValueExpr::ListAccess {
            target: Box::new(rewrite_aggregate_refs(target, aggregate_names, analyzed)),
            index: Box::new(rewrite_aggregate_refs(index, aggregate_names, analyzed)),
            span: *span,
        },
        ValueExpr::ListLiteral { items, span } => ValueExpr::ListLiteral {
            items: rewrite_exprs(items, aggregate_names, analyzed),
            span: *span,
        },
        ValueExpr::RecordLiteral { fields, span } => ValueExpr::RecordLiteral {
            fields: fields
                .iter()
                .map(|(key, field)| {
                    (
                        key.clone(),
                        rewrite_aggregate_refs(field, aggregate_names, analyzed),
                    )
                })
                .collect(),
            span: *span,
        },
        ValueExpr::BinaryOp { op, lhs, rhs, span } => ValueExpr::BinaryOp {
            op: *op,
            lhs: Box::new(rewrite_aggregate_refs(lhs, aggregate_names, analyzed)),
            rhs: Box::new(rewrite_aggregate_refs(rhs, aggregate_names, analyzed)),
            span: *span,
        },
        ValueExpr::UnaryOp { op, operand, span } => ValueExpr::UnaryOp {
            op: *op,
            operand: Box::new(rewrite_aggregate_refs(operand, aggregate_names, analyzed)),
            span: *span,
        },
        ValueExpr::FunctionCall {
            name,
            args,
            star,
            distinct,
            span,
        } => ValueExpr::FunctionCall {
            name: name.clone(),
            args: rewrite_exprs(args, aggregate_names, analyzed),
            star: *star,
            distinct: *distinct,
            span: *span,
        },
        ValueExpr::DurationBetween {
            start,
            end,
            qualifier,
            span,
        } => ValueExpr::DurationBetween {
            start: Box::new(rewrite_aggregate_refs(start, aggregate_names, analyzed)),
            end: Box::new(rewrite_aggregate_refs(end, aggregate_names, analyzed)),
            qualifier: *qualifier,
            span: *span,
        },
        ValueExpr::IsCheck {
            operand,
            kind,
            negated,
            span,
        } => ValueExpr::IsCheck {
            operand: Box::new(rewrite_aggregate_refs(operand, aggregate_names, analyzed)),
            kind: rewrite_is_check_kind(kind, aggregate_names, analyzed),
            negated: *negated,
            span: *span,
        },
        ValueExpr::InList {
            operand,
            list,
            negated,
            span,
        } => ValueExpr::InList {
            operand: Box::new(rewrite_aggregate_refs(operand, aggregate_names, analyzed)),
            list: rewrite_exprs(list, aggregate_names, analyzed),
            negated: *negated,
            span: *span,
        },
        ValueExpr::InListExpression {
            operand,
            list,
            negated,
            span,
        } => ValueExpr::InListExpression {
            operand: Box::new(rewrite_aggregate_refs(operand, aggregate_names, analyzed)),
            list: Box::new(rewrite_aggregate_refs(list, aggregate_names, analyzed)),
            negated: *negated,
            span: *span,
        },
        ValueExpr::AllDifferent { items, span } => ValueExpr::AllDifferent {
            items: rewrite_exprs(items, aggregate_names, analyzed),
            span: *span,
        },
        ValueExpr::Same { items, span } => ValueExpr::Same {
            items: rewrite_exprs(items, aggregate_names, analyzed),
            span: *span,
        },
        ValueExpr::PropertyExists { target, key, span } => ValueExpr::PropertyExists {
            target: Box::new(rewrite_aggregate_refs(target, aggregate_names, analyzed)),
            key: key.clone(),
            span: *span,
        },
        ValueExpr::Case {
            branches,
            else_branch,
            span,
        } => ValueExpr::Case {
            branches: branches
                .iter()
                .map(|(condition, result)| {
                    (
                        rewrite_aggregate_refs(condition, aggregate_names, analyzed),
                        rewrite_aggregate_refs(result, aggregate_names, analyzed),
                    )
                })
                .collect(),
            else_branch: else_branch
                .as_ref()
                .map(|result| Box::new(rewrite_aggregate_refs(result, aggregate_names, analyzed))),
            span: *span,
        },
        ValueExpr::Cast {
            value,
            target_type,
            span,
        } => ValueExpr::Cast {
            value: Box::new(rewrite_aggregate_refs(value, aggregate_names, analyzed)),
            target_type: target_type.clone(),
            span: *span,
        },
        ValueExpr::Normalize { source, form, span } => ValueExpr::Normalize {
            source: Box::new(rewrite_aggregate_refs(source, aggregate_names, analyzed)),
            form: *form,
            span: *span,
        },
        ValueExpr::Trim {
            spec,
            character,
            source,
            span,
        } => ValueExpr::Trim {
            spec: *spec,
            character: character.as_ref().map(|character| {
                Box::new(rewrite_aggregate_refs(character, aggregate_names, analyzed))
            }),
            source: Box::new(rewrite_aggregate_refs(source, aggregate_names, analyzed)),
            span: *span,
        },
    }
}

fn rewrite_exprs(
    values: &[ValueExpr],
    aggregate_names: &HashMap<ExprId, DbString>,
    analyzed: &AnalyzedStatement,
) -> Vec<ValueExpr> {
    values
        .iter()
        .map(|item| rewrite_aggregate_refs(item, aggregate_names, analyzed))
        .collect()
}

fn rewrite_is_check_kind(
    kind: &crate::IsCheckKind,
    aggregate_names: &HashMap<ExprId, DbString>,
    analyzed: &AnalyzedStatement,
) -> crate::IsCheckKind {
    match kind {
        crate::IsCheckKind::SourceOf(value) => crate::IsCheckKind::SourceOf(Box::new(
            rewrite_aggregate_refs(value, aggregate_names, analyzed),
        )),
        crate::IsCheckKind::DestinationOf(value) => crate::IsCheckKind::DestinationOf(Box::new(
            rewrite_aggregate_refs(value, aggregate_names, analyzed),
        )),
        crate::IsCheckKind::Null
        | crate::IsCheckKind::Directed
        | crate::IsCheckKind::Labeled(_)
        | crate::IsCheckKind::TruthValue(_)
        | crate::IsCheckKind::Typed(_)
        | crate::IsCheckKind::Normalized(_) => kind.clone(),
    }
}

fn group_key_alias(expr: &ValueExpr, items: &[ReturnItem]) -> Option<DbString> {
    items
        .iter()
        .find(|item| item.expr == *expr)
        .and_then(|item| column_name(&item.expr, item.alias.clone()))
        .or_else(|| column_name(expr, None))
}

fn column_name(expr: &ValueExpr, alias: Option<DbString>) -> Option<DbString> {
    alias.or(match expr {
        ValueExpr::Variable { name, .. } => Some(name.clone()),
        _ => None,
    })
}
