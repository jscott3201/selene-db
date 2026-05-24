//! Value-expression bind and type-inference handling.

use crate::{
    IsCheckKind, LimitValue, MatchClause, PipelineStatement, QueryPipeline, ReturnClause,
    ValueExpr,
    analyze::{
        error::{AnalysisError, ConditionClause},
        infer,
        types::{AnalyzedType, ExprId},
    },
};

use super::{ANALYZER_MAX_DEPTH, BindContext, pattern, query};
use crate::analyze::{binding::BindingUseKind, scope::ScopeKind};

pub(crate) fn bind_value_expr(
    ctx: &mut BindContext,
    expr: &ValueExpr,
) -> Result<ExprId, AnalysisError> {
    // Why: `ValueExpr` recursion fans out to one stack frame per nesting
    // level; the depth-256 contract enforced by `check_expr_depth` is well
    // above what macOS's 2 MB default thread stack can carry in debug
    // builds once enum-variant sizes grow. `stacker::maybe_grow` allocates
    // a fresh 1 MB segment whenever fewer than 64 KB remain, matching the
    // rustc/syn/serde analyzer pattern.
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || bind_value_expr_inner(ctx, expr))
}

fn bind_value_expr_inner(ctx: &mut BindContext, expr: &ValueExpr) -> Result<ExprId, AnalysisError> {
    if ctx.at_expr_root() {
        check_expr_depth(expr)?;
        check_subquery_depth(expr)?;
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
            ValueExpr::Normalize { source, .. } => {
                let source_id = bind_value_expr(ctx, source)?;
                infer::normalize(ctx.expr_type(source_id), source.span())?
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
            ValueExpr::ValueSubquery { body, span } => bind_value_subquery(ctx, body, *span)?,
            ValueExpr::Cast {
                value,
                target_type,
                span,
            } => {
                let value_id = bind_value_expr(ctx, value)?;
                infer::cast(target_type, ctx.expr_type(value_id), *span)?
            }
        };
        Ok(ctx.allocate_expr(expr, ty))
    })
}

fn bind_value_subquery(
    ctx: &mut BindContext,
    body: &QueryPipeline,
    span: crate::SourceSpan,
) -> Result<AnalyzedType, AnalysisError> {
    validate_value_subquery_shape(body, span)?;
    let mut body = body.clone();
    ctx.with_child_scope(ScopeKind::Subquery, span, false, |ctx| {
        query::bind_query_pipeline(ctx, &mut body)
    })?;
    Ok(AnalyzedType::DYNAMIC)
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
            ValueExpr::Cast { value, .. } | ValueExpr::Normalize { source: value, .. } => {
                stack.push((value, next));
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

fn check_subquery_depth(expr: &ValueExpr) -> Result<(), AnalysisError> {
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
            PipelineStatement::Unwind(unwind) => check_expr_subquery_depth(&unwind.source, depth)?,
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
            ValueExpr::ListAccess { target, index, .. } => {
                stack.push((index, depth));
                stack.push((target, depth));
            }
            ValueExpr::ListLiteral { items, .. }
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
            ValueExpr::Like {
                operand, pattern, ..
            } => {
                stack.push((pattern, depth));
                stack.push((operand, depth));
            }
            ValueExpr::Between {
                operand, low, high, ..
            } => {
                stack.push((high, depth));
                stack.push((low, depth));
                stack.push((operand, depth));
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
            ValueExpr::Exists { pattern, .. } | ValueExpr::CountSubquery { pattern, .. } => {
                check_match_clause_subquery_depth(pattern, depth.saturating_add(1))?;
            }
            ValueExpr::Cast { value, .. } => stack.push((value, depth)),
            ValueExpr::Normalize { source, .. } => stack.push((source, depth)),
            ValueExpr::Literal(_) | ValueExpr::Variable { .. } | ValueExpr::Parameter { .. } => {}
        }
    }
    Ok(())
}

fn validate_value_subquery_shape(
    body: &QueryPipeline,
    span: crate::SourceSpan,
) -> Result<(), AnalysisError> {
    let Some(return_index) = body
        .statements
        .iter()
        .rposition(|statement| matches!(statement, PipelineStatement::Return(_)))
    else {
        return Err(value_shape_error(
            "VALUE subquery requires a RETURN clause (ISO §20.6; iso_gql_clause20.txt:1143)",
            span,
        ));
    };
    let PipelineStatement::Return(return_clause) = &body.statements[return_index] else {
        unreachable!("rposition selected a RETURN");
    };
    if return_clause.star || return_clause.items.len() != 1 {
        return Err(value_shape_error(
            "VALUE subquery RETURN must contain exactly one item (ISO §20.6; iso_gql_clause20.txt:1143)",
            return_clause.span,
        ));
    }
    if has_effective_final_limit_one(body, return_index)
        || direct_aggregate_without_group_by(return_clause)
    {
        return Ok(());
    }
    Err(value_shape_error(
        "VALUE subquery must use LIMIT 1 or a direct aggregate without GROUP BY (ISO §20.6; iso_gql_clause20.txt:1143)",
        return_clause.span,
    ))
}

fn direct_aggregate_without_group_by(clause: &ReturnClause) -> bool {
    clause.group_by.is_none()
        && clause.items.first().is_some_and(|item| {
            matches!(
                &item.expr,
                ValueExpr::FunctionCall { name, .. } if is_aggregate_name(name.first())
            )
        })
}

fn has_effective_final_limit_one(body: &QueryPipeline, return_index: usize) -> bool {
    body.statements
        .iter()
        .skip(return_index.saturating_add(1))
        .try_fold(false, |seen_limit_one, statement| match statement {
            PipelineStatement::Limit(limit) => Some(matches!(limit, LimitValue::Count(1, _))),
            PipelineStatement::Offset(_) | PipelineStatement::Sorting(_) => Some(seen_limit_one),
            _ => None,
        })
        .unwrap_or(false)
}

fn is_aggregate_name(name: &selene_core::IStr) -> bool {
    let name = name.as_str();
    [
        "count",
        "sum",
        "average",
        "avg",
        "min",
        "max",
        "collect",
        "collect_list",
        "stddev_pop",
        "stddev_samp",
    ]
    .iter()
    .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

fn value_shape_error(message: &'static str, span: crate::SourceSpan) -> AnalysisError {
    AnalysisError::ValueSubqueryShapeViolation {
        message: message.to_owned(),
        span,
    }
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
