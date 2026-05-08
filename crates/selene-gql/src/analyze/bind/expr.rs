//! Value-expression bind handling.

use crate::{
    IsCheckKind, Literal, ValueExpr,
    analyze::{binding::BindingUseKind, error::AnalysisError, scope::ScopeKind},
};

use super::{BindContext, pattern};

pub(crate) fn bind_value_expr(
    ctx: &mut BindContext,
    expr: &ValueExpr,
) -> Result<(), AnalysisError> {
    match expr {
        ValueExpr::Literal(literal) => bind_literal(ctx, literal),
        ValueExpr::Variable { name, span } => {
            ctx.resolve(*name, *span, BindingUseKind::Variable)?;
            Ok(())
        }
        ValueExpr::Parameter { .. } => Ok(()),
        ValueExpr::PropertyAccess { target, .. } => bind_value_expr(ctx, target),
        ValueExpr::ListAccess { target, index, .. } => {
            bind_value_expr(ctx, target)?;
            bind_value_expr(ctx, index)
        }
        ValueExpr::ListLiteral { items, .. } => bind_many(ctx, items),
        ValueExpr::RecordLiteral { fields, .. } => {
            for (_, value) in fields {
                bind_value_expr(ctx, value)?;
            }
            Ok(())
        }
        ValueExpr::BinaryOp { lhs, rhs, .. } => {
            bind_value_expr(ctx, lhs)?;
            bind_value_expr(ctx, rhs)
        }
        ValueExpr::UnaryOp { operand, .. } => bind_value_expr(ctx, operand),
        ValueExpr::FunctionCall { args, .. } => bind_many(ctx, args),
        ValueExpr::IsCheck { operand, kind, .. } => {
            bind_value_expr(ctx, operand)?;
            bind_is_check(ctx, kind)
        }
        ValueExpr::InList { operand, list, .. } => {
            bind_value_expr(ctx, operand)?;
            bind_many(ctx, list)
        }
        ValueExpr::Like {
            operand, pattern, ..
        } => {
            bind_value_expr(ctx, operand)?;
            bind_value_expr(ctx, pattern)
        }
        ValueExpr::Between {
            operand, low, high, ..
        } => {
            bind_value_expr(ctx, operand)?;
            bind_value_expr(ctx, low)?;
            bind_value_expr(ctx, high)
        }
        ValueExpr::AllDifferent { items, .. } | ValueExpr::Same { items, .. } => {
            bind_many(ctx, items)
        }
        ValueExpr::PropertyExists { target, .. } => bind_value_expr(ctx, target),
        ValueExpr::Case {
            branches,
            else_branch,
            span,
        } => {
            for (condition, value) in branches {
                ctx.with_child_scope(ScopeKind::CaseBranch, *span, false, |ctx| {
                    bind_value_expr(ctx, condition)?;
                    bind_value_expr(ctx, value)
                })?;
            }
            if let Some(value) = else_branch {
                ctx.with_child_scope(ScopeKind::CaseBranch, value.span(), false, |ctx| {
                    bind_value_expr(ctx, value)
                })?;
            }
            Ok(())
        }
        ValueExpr::Exists {
            pattern: clause,
            span,
            ..
        }
        | ValueExpr::CountSubquery {
            pattern: clause,
            span,
        } => ctx.with_child_scope(ScopeKind::Subquery, *span, false, |ctx| {
            pattern::bind_match_clause(ctx, clause)
        }),
    }
}

fn bind_literal(_ctx: &mut BindContext, literal: &Literal) -> Result<(), AnalysisError> {
    match literal {
        Literal::Bool(..)
        | Literal::Integer(..)
        | Literal::Float(..)
        | Literal::String(..)
        | Literal::Null(..) => Ok(()),
    }
}

fn bind_many(ctx: &mut BindContext, values: &[ValueExpr]) -> Result<(), AnalysisError> {
    for value in values {
        bind_value_expr(ctx, value)?;
    }
    Ok(())
}

fn bind_is_check(ctx: &mut BindContext, kind: &IsCheckKind) -> Result<(), AnalysisError> {
    match kind {
        IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) => {
            bind_value_expr(ctx, value)
        }
        IsCheckKind::Null
        | IsCheckKind::Directed
        | IsCheckKind::Labeled(_)
        | IsCheckKind::TruthValue(_)
        | IsCheckKind::Typed(_)
        | IsCheckKind::Normalized(_) => Ok(()),
    }
}
