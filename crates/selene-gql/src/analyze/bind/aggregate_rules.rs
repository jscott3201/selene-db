//! Aggregate-specific analyzer rules.

use crate::{ReturnItem, ValueExpr, analyze::error::AnalysisError, ast::eq::value_structurally_eq};

use super::expr;

pub(crate) fn contains_aggregate_function(value: &ValueExpr) -> bool {
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        if matches!(
            value,
            ValueExpr::FunctionCall { name, .. } if expr::is_aggregate_name(name.first())
        ) {
            return true;
        }
        value.for_each_child(&mut |child| stack.push(child));
    }
    false
}

/// Validate ISO 20.9 aggregate syntax rules that need AST context after
/// expression binding.
pub(crate) fn validate_aggregate_nesting(
    items: &[ReturnItem],
    having: Option<&ValueExpr>,
) -> Result<(), AnalysisError> {
    for item in items {
        validate_aggregate_nesting_in_expr(&item.expr)?;
    }
    if let Some(having) = having {
        validate_aggregate_nesting_in_expr(having)?;
    }
    Ok(())
}

/// Validate ISO 14.11 grouped projection item rules.
pub(crate) fn validate_grouped_projection_items(
    items: &[ReturnItem],
    group_by: Option<&[ValueExpr]>,
) -> Result<(), AnalysisError> {
    let Some(group_by) = group_by else {
        return Ok(());
    };

    for item in items {
        if contains_aggregate_function(&item.expr)
            || group_by
                .iter()
                .any(|key| value_structurally_eq(key, &item.expr))
        {
            continue;
        }
        return Err(AnalysisError::GroupedProjectionItemNotGrouped { span: item.span });
    }

    Ok(())
}

fn validate_aggregate_nesting_in_expr(value: &ValueExpr) -> Result<(), AnalysisError> {
    let mut stack = vec![(value, false)];
    while let Some((value, inside_aggregate)) = stack.pop() {
        match value {
            ValueExpr::FunctionCall { name, args, .. } if expr::is_aggregate_name(name.first()) => {
                if inside_aggregate {
                    return Err(AnalysisError::AggregateNestingViolation {
                        message: "aggregate function cannot contain another aggregate function"
                            .to_owned(),
                        span: value.span(),
                    });
                }
                stack.extend(args.iter().map(|arg| (arg, true)));
            }
            ValueExpr::PropertyAccess { target, .. }
            | ValueExpr::UnaryOp {
                operand: target, ..
            }
            | ValueExpr::PropertyExists { target, .. }
            | ValueExpr::Cast { value: target, .. }
            | ValueExpr::Normalize { source: target, .. } => stack.push((target, inside_aggregate)),
            ValueExpr::ListLiteral { items, .. }
            | ValueExpr::PathConstructor {
                elements: items, ..
            }
            | ValueExpr::AllDifferent { items, .. }
            | ValueExpr::Same { items, .. }
            | ValueExpr::FunctionCall { args: items, .. } => {
                stack.extend(items.iter().map(|item| (item, inside_aggregate)));
            }
            ValueExpr::DurationBetween { start, end, .. } => {
                stack.push((end, inside_aggregate));
                stack.push((start, inside_aggregate));
            }
            ValueExpr::RecordLiteral { fields, .. } => {
                stack.extend(fields.iter().map(|(_, field)| (field, inside_aggregate)));
            }
            ValueExpr::BinaryOp { lhs, rhs, .. } => {
                stack.push((rhs, inside_aggregate));
                stack.push((lhs, inside_aggregate));
            }
            ValueExpr::IsCheck { operand, kind, .. } => {
                stack.push((operand, inside_aggregate));
                match kind {
                    crate::IsCheckKind::SourceOf(value)
                    | crate::IsCheckKind::DestinationOf(value) => {
                        stack.push((value, inside_aggregate));
                    }
                    crate::IsCheckKind::Null
                    | crate::IsCheckKind::Directed
                    | crate::IsCheckKind::Labeled(_)
                    | crate::IsCheckKind::TruthValue(_)
                    | crate::IsCheckKind::Typed(_)
                    | crate::IsCheckKind::Normalized(_) => {}
                }
            }
            ValueExpr::InList { operand, list, .. } => {
                stack.extend(list.iter().map(|item| (item, inside_aggregate)));
                stack.push((operand, inside_aggregate));
            }
            ValueExpr::InListExpression { operand, list, .. } => {
                stack.push((list, inside_aggregate));
                stack.push((operand, inside_aggregate));
            }
            ValueExpr::Case {
                branches,
                else_branch,
                ..
            } => {
                if let Some(value) = else_branch {
                    stack.push((value, inside_aggregate));
                }
                for (condition, result) in branches {
                    stack.push((result, inside_aggregate));
                    stack.push((condition, inside_aggregate));
                }
            }
            ValueExpr::Trim {
                character, source, ..
            } => {
                stack.push((source, inside_aggregate));
                if let Some(character) = character {
                    stack.push((character, inside_aggregate));
                }
            }
            ValueExpr::Literal(_)
            | ValueExpr::Variable { .. }
            | ValueExpr::Parameter { .. }
            | ValueExpr::Exists { .. }
            | ValueExpr::ValueSubquery { .. } => {}
        }
    }
    Ok(())
}
