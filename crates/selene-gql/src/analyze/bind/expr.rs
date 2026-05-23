//! Value-expression bind and type-inference handling.

use crate::{
    IsCheckKind, ValueExpr,
    analyze::{
        error::{AnalysisError, ConditionClause},
        infer,
        types::{AnalyzedType, ExprId},
    },
};

use super::{ANALYZER_MAX_DEPTH, BindContext, pattern};
use crate::analyze::{binding::BindingUseKind, scope::ScopeKind};

pub(crate) fn bind_value_expr(
    ctx: &mut BindContext,
    expr: &ValueExpr,
) -> Result<ExprId, AnalysisError> {
    if ctx.at_expr_root() {
        check_expr_depth(expr)?;
    }
    ctx.with_expr_depth(|ctx| {
        let ty = match expr {
            ValueExpr::Literal(literal) => infer::literal(literal),
            ValueExpr::Variable { name, span } => {
                let binding = ctx.resolve(*name, *span, BindingUseKind::Variable)?;
                ctx.binding_type(binding)
            }
            ValueExpr::Parameter { .. } => AnalyzedType::Dynamic,
            ValueExpr::PropertyAccess { target, span, .. } => {
                let target_id = bind_value_expr(ctx, target)?;
                reject_group_variable_property_access(ctx.expr_type(target_id), *span)?;
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
            ValueExpr::PropertyExists { target, span, .. } => {
                let target_id = bind_value_expr(ctx, target)?;
                reject_group_variable_property_access(ctx.expr_type(target_id), *span)?;
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
            ValueExpr::ValueSubquery { span, .. } => {
                return Err(AnalysisError::NotImplemented {
                    message: "VALUE { ... } subqueries land in BRIEF-134 commit 2".into(),
                    span: *span,
                    hint: None,
                });
            }
        };
        Ok(ctx.allocate_expr(expr, ty))
    })
}

fn check_expr_depth(expr: &ValueExpr) -> Result<(), AnalysisError> {
    let mut stack = vec![(expr, 1_u32)];
    while let Some((expr, depth)) = stack.pop() {
        if depth > ANALYZER_MAX_DEPTH {
            return Err(AnalysisError::RecursionLimitExceeded { depth });
        }
        let next = depth.saturating_add(1);
        match expr {
            ValueExpr::PropertyAccess { target, .. } => stack.push((target, next)),
            ValueExpr::ListAccess { target, index, .. } => {
                stack.push((index, next));
                stack.push((target, next));
            }
            ValueExpr::ListLiteral { items, .. } => {
                stack.extend(items.iter().rev().map(|item| (item, next)));
            }
            ValueExpr::RecordLiteral { fields, .. } => {
                stack.extend(fields.iter().rev().map(|(_, value)| (value, next)));
            }
            ValueExpr::BinaryOp { lhs, rhs, .. } => {
                stack.push((rhs, next));
                stack.push((lhs, next));
            }
            ValueExpr::UnaryOp { operand, .. } => stack.push((operand, next)),
            ValueExpr::FunctionCall { args, .. } => {
                stack.extend(args.iter().rev().map(|arg| (arg, next)));
            }
            ValueExpr::IsCheck { operand, kind, .. } => {
                stack.push((operand, next));
                match kind {
                    IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) => {
                        stack.push((value, next));
                    }
                    IsCheckKind::Null
                    | IsCheckKind::Directed
                    | IsCheckKind::Labeled(_)
                    | IsCheckKind::TruthValue(_)
                    | IsCheckKind::Typed(_)
                    | IsCheckKind::Normalized(_) => {}
                }
            }
            ValueExpr::InList { operand, list, .. } => {
                stack.extend(list.iter().rev().map(|item| (item, next)));
                stack.push((operand, next));
            }
            ValueExpr::Like {
                operand, pattern, ..
            } => {
                stack.push((pattern, next));
                stack.push((operand, next));
            }
            ValueExpr::Between {
                operand, low, high, ..
            } => {
                stack.push((high, next));
                stack.push((low, next));
                stack.push((operand, next));
            }
            ValueExpr::AllDifferent { items, .. } | ValueExpr::Same { items, .. } => {
                stack.extend(items.iter().rev().map(|item| (item, next)));
            }
            ValueExpr::PropertyExists { target, .. } => stack.push((target, next)),
            ValueExpr::Case {
                branches,
                else_branch,
                ..
            } => {
                if let Some(value) = else_branch {
                    stack.push((value, next));
                }
                for (condition, value) in branches.iter().rev() {
                    stack.push((value, next));
                    stack.push((condition, next));
                }
            }
            ValueExpr::Literal(_)
            | ValueExpr::Variable { .. }
            | ValueExpr::Parameter { .. }
            | ValueExpr::Exists { .. }
            | ValueExpr::CountSubquery { .. }
            | ValueExpr::ValueSubquery { .. } => {}
        }
    }
    Ok(())
}

fn reject_group_variable_property_access(
    ty: &AnalyzedType,
    span: crate::SourceSpan,
) -> Result<(), AnalysisError> {
    let AnalyzedType::Resolved(crate::GqlType::List(item)) = ty else {
        return Ok(());
    };
    if matches!(
        item.as_ref(),
        crate::GqlType::NodeRef | crate::GqlType::EdgeRef
    ) {
        return Err(AnalysisError::NotImplemented {
            message: "group-variable property access is not supported".into(),
            span,
            hint: Some(
                "return the group variable as a list, or unnest it before accessing element properties"
                    .into(),
            ),
        });
    }
    Ok(())
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
