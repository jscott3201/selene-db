//! Binding-reference collection for planned expressions.

use crate::{
    IsCheckKind, SourceSpan, ValueExpr,
    analyze::{AnalyzedStatement, BindingId},
    plan::PlannerError,
};

use super::expr::{outer_binding_uses_in_match, outer_binding_uses_in_span};

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
        ValueExpr::ValueSubquery { body, .. } => {
            refs.extend(
                outer_binding_uses_in_span(body.span, body.span, analyzed)?
                    .into_iter()
                    .map(|(binding, _, span)| (binding, span)),
            );
        }
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
