//! Expression and subquery depth guards for the bind pass.

use crate::{
    IsCheckKind, MatchClause, PipelineStatement, QueryPipeline, ReturnClause, ValueExpr,
    analyze::error::AnalysisError,
};

use super::ANALYZER_MAX_DEPTH;

pub(super) fn check_expr_depth(expr: &ValueExpr) -> Result<(), AnalysisError> {
    let mut stack = vec![(expr, 1_u32)];
    while let Some((expr, depth)) = stack.pop() {
        if depth > ANALYZER_MAX_DEPTH {
            return Err(AnalysisError::RecursionLimitExceeded { depth });
        }
        let next = depth.saturating_add(1);
        match expr {
            ValueExpr::PropertyAccess { target, .. } => stack.push((target, next)),
            ValueExpr::ListLiteral { items, .. } => {
                stack.extend(items.iter().rev().map(|item| (item, next)));
            }
            ValueExpr::PathConstructor { elements, .. } => {
                stack.extend(elements.iter().rev().map(|element| (element, next)));
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
            ValueExpr::DurationBetween { start, end, .. } => {
                stack.push((end, next));
                stack.push((start, next));
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
            ValueExpr::InListExpression { operand, list, .. } => {
                stack.push((list, next));
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
            ValueExpr::Cast { value, .. } | ValueExpr::Normalize { source: value, .. } => {
                stack.push((value, next));
            }
            ValueExpr::Trim {
                character, source, ..
            } => {
                stack.push((source, next));
                if let Some(character) = character {
                    stack.push((character, next));
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

pub(super) fn check_subquery_depth(expr: &ValueExpr) -> Result<(), AnalysisError> {
    check_expr_subquery_depth(expr, 1)
}

pub(crate) fn check_query_subquery_depth(
    pipeline: &QueryPipeline,
    depth: u32,
) -> Result<(), AnalysisError> {
    if depth > ANALYZER_MAX_DEPTH {
        return Err(AnalysisError::RecursionLimitExceeded { depth });
    }
    for statement in &pipeline.statements {
        match statement {
            PipelineStatement::Match(clause) => {
                check_match_clause_subquery_depth(clause, depth)?;
            }
            PipelineStatement::Filter(value) => check_expr_subquery_depth(value, depth)?,
            PipelineStatement::Let(bindings) => {
                for binding in bindings {
                    check_expr_subquery_depth(&binding.value, depth)?;
                }
            }
            PipelineStatement::For(statement) => {
                check_expr_subquery_depth(&statement.source, depth)?
            }
            PipelineStatement::Sorting(terms) => {
                for term in terms {
                    check_expr_subquery_depth(&term.expr, depth)?;
                }
            }
            PipelineStatement::Return(clause) => check_return_subquery_depth(clause, depth)?,
            PipelineStatement::With(clause) => {
                for item in &clause.items {
                    check_expr_subquery_depth(&item.expr, depth)?;
                }
                if let Some(group_by) = &clause.group_by {
                    for value in group_by {
                        check_expr_subquery_depth(value, depth)?;
                    }
                }
                if let Some(value) = &clause.having {
                    check_expr_subquery_depth(value, depth)?;
                }
                if let Some(value) = &clause.where_clause {
                    check_expr_subquery_depth(value, depth)?;
                }
            }
            PipelineStatement::Call(call) => {
                for arg in &call.args {
                    check_expr_subquery_depth(arg, depth)?;
                }
                if let Some(filter) = &call.yield_filter {
                    check_expr_subquery_depth(filter, depth)?;
                }
            }
            PipelineStatement::CallSubquery(call) => {
                check_query_subquery_depth(&call.body, depth.saturating_add(1))?;
            }
            PipelineStatement::Limit(_) | PipelineStatement::Offset(_) => {}
        }
    }
    Ok(())
}

fn check_match_clause_subquery_depth(
    clause: &MatchClause,
    depth: u32,
) -> Result<(), AnalysisError> {
    for pattern in &clause.patterns {
        for element in &pattern.elements {
            match element {
                crate::PatternElement::Node(node) => {
                    for (_, value) in &node.properties {
                        check_expr_subquery_depth(value, depth)?;
                    }
                    if let Some(value) = &node.inline_where {
                        check_expr_subquery_depth(value, depth)?;
                    }
                }
                crate::PatternElement::Edge(edge) => {
                    for (_, value) in &edge.properties {
                        check_expr_subquery_depth(value, depth)?;
                    }
                    if let Some(value) = &edge.inline_where {
                        check_expr_subquery_depth(value, depth)?;
                    }
                }
            }
        }
    }
    if let Some(value) = &clause.where_clause {
        check_expr_subquery_depth(value, depth)?;
    }
    Ok(())
}

fn check_return_subquery_depth(clause: &ReturnClause, depth: u32) -> Result<(), AnalysisError> {
    for item in &clause.items {
        check_expr_subquery_depth(&item.expr, depth)?;
    }
    if let Some(group_by) = &clause.group_by {
        for value in group_by {
            check_expr_subquery_depth(value, depth)?;
        }
    }
    if let Some(value) = &clause.having {
        check_expr_subquery_depth(value, depth)?;
    }
    Ok(())
}

fn check_expr_subquery_depth(expr: &ValueExpr, depth: u32) -> Result<(), AnalysisError> {
    let mut stack = vec![(expr, depth)];
    while let Some((expr, depth)) = stack.pop() {
        match expr {
            ValueExpr::PropertyAccess { target, .. } => stack.push((target, depth)),
            ValueExpr::ListLiteral { items, .. }
            | ValueExpr::PathConstructor {
                elements: items, ..
            }
            | ValueExpr::AllDifferent { items, .. }
            | ValueExpr::Same { items, .. } => {
                stack.extend(items.iter().rev().map(|item| (item, depth)));
            }
            ValueExpr::RecordLiteral { fields, .. } => {
                stack.extend(fields.iter().rev().map(|(_, value)| (value, depth)));
            }
            ValueExpr::BinaryOp { lhs, rhs, .. } => {
                stack.push((rhs, depth));
                stack.push((lhs, depth));
            }
            ValueExpr::UnaryOp { operand, .. }
            | ValueExpr::PropertyExists {
                target: operand, ..
            } => stack.push((operand, depth)),
            ValueExpr::FunctionCall { args, .. } | ValueExpr::InList { list: args, .. } => {
                stack.extend(args.iter().rev().map(|arg| (arg, depth)));
                if let ValueExpr::InList { operand, .. } = expr {
                    stack.push((operand, depth));
                }
            }
            ValueExpr::DurationBetween { start, end, .. } => {
                stack.push((end, depth));
                stack.push((start, depth));
            }
            ValueExpr::InListExpression { operand, list, .. } => {
                stack.push((list, depth));
                stack.push((operand, depth));
            }
            ValueExpr::IsCheck { operand, kind, .. } => {
                stack.push((operand, depth));
                match kind {
                    IsCheckKind::SourceOf(value) | IsCheckKind::DestinationOf(value) => {
                        stack.push((value, depth));
                    }
                    IsCheckKind::Null
                    | IsCheckKind::Directed
                    | IsCheckKind::Labeled(_)
                    | IsCheckKind::TruthValue(_)
                    | IsCheckKind::Typed(_)
                    | IsCheckKind::Normalized(_) => {}
                }
            }
            ValueExpr::Case {
                branches,
                else_branch,
                ..
            } => {
                if let Some(value) = else_branch {
                    stack.push((value, depth));
                }
                for (condition, value) in branches.iter().rev() {
                    stack.push((value, depth));
                    stack.push((condition, depth));
                }
            }
            ValueExpr::ValueSubquery { body, .. } => {
                check_query_subquery_depth(body, depth.saturating_add(1))?;
            }
            ValueExpr::Exists { pattern, .. } => {
                check_match_clause_subquery_depth(pattern, depth.saturating_add(1))?;
            }
            ValueExpr::Cast { value, .. } => stack.push((value, depth)),
            ValueExpr::Normalize { source, .. } => stack.push((source, depth)),
            ValueExpr::Trim {
                character, source, ..
            } => {
                stack.push((source, depth));
                if let Some(character) = character {
                    stack.push((character, depth));
                }
            }
            ValueExpr::Literal(_) | ValueExpr::Variable { .. } | ValueExpr::Parameter { .. } => {}
        }
    }
    Ok(())
}
