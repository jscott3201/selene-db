//! Value-expression bind and type-inference handling.

use crate::{
    IsCheckKind, ValueExpr,
    analyze::{
        error::{AnalysisError, ConditionClause},
        infer,
        types::{AnalyzedType, ExprId},
    },
};

use super::{BindContext, pattern};
use crate::analyze::{binding::BindingUseKind, scope::ScopeKind};

pub(crate) fn bind_value_expr(
    ctx: &mut BindContext,
    expr: &ValueExpr,
) -> Result<ExprId, AnalysisError> {
    let ty = match expr {
        ValueExpr::Literal(literal) => infer::literal(literal),
        ValueExpr::Variable { name, span } => {
            let binding = ctx.resolve(*name, *span, BindingUseKind::Variable)?;
            ctx.binding_type(binding)
        }
        ValueExpr::Parameter { .. } => AnalyzedType::Dynamic,
        ValueExpr::PropertyAccess { target, .. } => {
            bind_value_expr(ctx, target)?;
            AnalyzedType::Dynamic
        }
        ValueExpr::ListAccess { target, index, .. } => {
            bind_value_expr(ctx, target)?;
            bind_value_expr(ctx, index)?;
            AnalyzedType::Dynamic
        }
        ValueExpr::ListLiteral { items, .. } => {
            let item_types = bind_many_with_spans(ctx, items)?;
            infer::list_literal(&item_types)?
        }
        ValueExpr::RecordLiteral { fields, .. } => {
            for (_, value) in fields {
                bind_value_expr(ctx, value)?;
            }
            // Why: GV45-GV48 (RECORD types) are demoted to
            // NOT_SUPPORTED_RATIONALE per CLAUDE.md D2 amendment
            // (2026-05-08). The parser is the gate for whether a
            // RecordLiteral reaches the analyzer; leave the type cell Dynamic
            // rather than implying claimed-feature support.
            AnalyzedType::Dynamic
        }
        ValueExpr::BinaryOp { op, lhs, rhs, .. } => {
            let lhs_id = bind_value_expr(ctx, lhs)?;
            let rhs_id = bind_value_expr(ctx, rhs)?;
            infer::binary(
                *op,
                ctx.expr_type(lhs_id),
                lhs.span(),
                ctx.expr_type(rhs_id),
                rhs.span(),
            )?
        }
        ValueExpr::UnaryOp { op, operand, .. } => {
            let operand_id = bind_value_expr(ctx, operand)?;
            infer::unary(*op, ctx.expr_type(operand_id), operand.span())?
        }
        ValueExpr::FunctionCall { args, .. } => {
            bind_many(ctx, args)?;
            AnalyzedType::Dynamic
        }
        ValueExpr::IsCheck {
            operand,
            kind,
            span,
            ..
        } => {
            let operand_id = bind_value_expr(ctx, operand)?;
            bind_is_check(ctx, kind)?;
            infer::is_check(kind, ctx.expr_type(operand_id), operand.span(), *span)?
        }
        ValueExpr::InList { operand, list, .. } => {
            let operand_id = bind_value_expr(ctx, operand)?;
            let items = bind_many_with_spans(ctx, list)?;
            infer::in_list(ctx.expr_type(operand_id), operand.span(), &items)?
        }
        ValueExpr::Like {
            operand, pattern, ..
        } => {
            let operand_id = bind_value_expr(ctx, operand)?;
            let pattern_id = bind_value_expr(ctx, pattern)?;
            infer::like(
                ctx.expr_type(operand_id),
                operand.span(),
                ctx.expr_type(pattern_id),
                pattern.span(),
            )?
        }
        ValueExpr::Between {
            operand, low, high, ..
        } => {
            let operand_id = bind_value_expr(ctx, operand)?;
            let low_id = bind_value_expr(ctx, low)?;
            let high_id = bind_value_expr(ctx, high)?;
            infer::between(
                ctx.expr_type(operand_id),
                operand.span(),
                ctx.expr_type(low_id),
                low.span(),
                ctx.expr_type(high_id),
                high.span(),
            )?
        }
        ValueExpr::AllDifferent { items, .. } | ValueExpr::Same { items, .. } => {
            bind_many(ctx, items)?;
            AnalyzedType::Resolved(crate::GqlType::Boolean)
        }
        ValueExpr::PropertyExists { target, .. } => {
            bind_value_expr(ctx, target)?;
            AnalyzedType::Resolved(crate::GqlType::Boolean)
        }
        ValueExpr::Case {
            branches,
            else_branch,
            span,
        } => {
            let mut result_types =
                Vec::with_capacity(branches.len() + usize::from(else_branch.is_some()));
            for (condition, value) in branches {
                let value_id =
                    ctx.with_child_scope(ScopeKind::CaseBranch, *span, false, |ctx| {
                        bind_condition(ctx, condition, ConditionClause::CaseWhen)?;
                        bind_value_expr(ctx, value)
                    })?;
                result_types.push((ctx.expr_type(value_id).clone(), value.span()));
            }
            if let Some(value) = else_branch {
                let value_id =
                    ctx.with_child_scope(ScopeKind::CaseBranch, value.span(), false, |ctx| {
                        bind_value_expr(ctx, value)
                    })?;
                result_types.push((ctx.expr_type(value_id).clone(), value.span()));
            }
            infer::case_result(&result_types)?
        }
        ValueExpr::Exists {
            pattern: clause,
            span,
            ..
        } => {
            ctx.with_child_scope(ScopeKind::Subquery, *span, false, |ctx| {
                pattern::bind_match_clause(ctx, clause)
            })?;
            AnalyzedType::Resolved(crate::GqlType::Boolean)
        }
        ValueExpr::CountSubquery {
            pattern: clause,
            span,
        } => {
            ctx.with_child_scope(ScopeKind::Subquery, *span, false, |ctx| {
                pattern::bind_match_clause(ctx, clause)
            })?;
            AnalyzedType::Resolved(crate::GqlType::Integer)
        }
    };
    Ok(ctx.allocate_expr(expr, ty))
}

pub(crate) fn bind_condition(
    ctx: &mut BindContext,
    expr: &ValueExpr,
    clause: ConditionClause,
) -> Result<ExprId, AnalysisError> {
    let id = bind_value_expr(ctx, expr)?;
    infer::condition(ctx.expr_type(id), expr.span(), clause)?;
    Ok(id)
}

fn bind_many(
    ctx: &mut BindContext,
    values: &[ValueExpr],
) -> Result<Vec<AnalyzedType>, AnalysisError> {
    values
        .iter()
        .map(|value| {
            let id = bind_value_expr(ctx, value)?;
            Ok(ctx.expr_type(id).clone())
        })
        .collect()
}

fn bind_many_with_spans(
    ctx: &mut BindContext,
    values: &[ValueExpr],
) -> Result<Vec<(AnalyzedType, crate::SourceSpan)>, AnalysisError> {
    values
        .iter()
        .map(|value| {
            let id = bind_value_expr(ctx, value)?;
            Ok((ctx.expr_type(id).clone(), value.span()))
        })
        .collect()
}

fn bind_is_check(ctx: &mut BindContext, kind: &IsCheckKind) -> Result<(), AnalysisError> {
    match kind {
        IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) => {
            bind_value_expr(ctx, value)?;
            Ok(())
        }
        IsCheckKind::Null
        | IsCheckKind::Directed
        | IsCheckKind::Labeled(_)
        | IsCheckKind::TruthValue(_)
        | IsCheckKind::Typed(_)
        | IsCheckKind::Normalized(_) => Ok(()),
    }
}
