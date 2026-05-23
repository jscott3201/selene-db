//! Read-query bind handling.

use crate::{
    InlineProcedureCall, LetBinding, LimitValue, OrderTerm, PipelineStatement, ProcedureMutability,
    QueryPipeline, ReturnClause, ReturnItem, UnwindStatement, ValueExpr, WithClause, YieldColumn,
    analyze::{
        BindingDeclKind,
        error::{AnalysisError, ConditionClause},
        types::AnalyzedType,
    },
};

use super::{BindContext, call, expr, pattern};
use crate::analyze::scope::ScopeKind;

pub(crate) fn bind_query_pipeline(
    ctx: &mut BindContext,
    pipeline: &mut QueryPipeline,
) -> Result<(), AnalysisError> {
    for statement in &mut pipeline.statements {
        bind_pipeline_statement(ctx, statement)?;
    }
    Ok(())
}

pub(crate) fn bind_pipeline_statement(
    ctx: &mut BindContext,
    statement: &mut PipelineStatement,
) -> Result<(), AnalysisError> {
    match statement {
        PipelineStatement::Match(clause) => pattern::bind_match_clause(ctx, clause),
        PipelineStatement::Filter(value) => {
            expr::bind_condition(ctx, value, ConditionClause::Filter)?;
            Ok(())
        }
        PipelineStatement::Let(bindings) => bind_let(ctx, bindings),
        PipelineStatement::Unwind(unwind) => bind_unwind(ctx, unwind),
        PipelineStatement::Sorting(terms) => bind_sorting(ctx, terms),
        PipelineStatement::Limit(value) | PipelineStatement::Offset(value) => {
            bind_limit_value(value);
            Ok(())
        }
        PipelineStatement::Return(clause) => bind_return_clause(ctx, clause),
        PipelineStatement::With(clause) => bind_with_clause(ctx, clause),
        PipelineStatement::Call(call) => {
            let metadata = call::lookup_metadata(ctx, call)?;
            if matches!(
                metadata.mutability,
                ProcedureMutability::GraphWrite
                    | ProcedureMutability::SchemaWrite
                    | ProcedureMutability::Admin
            ) {
                return Err(AnalysisError::MutatingProcedureInReadPipeline {
                    procedure: call.name.clone().into_vec().into_boxed_slice(),
                    mutability: metadata.mutability,
                    span: call.span,
                });
            }
            call::bind_procedure_call_with_metadata(ctx, call, metadata)?;
            Ok(())
        }
        PipelineStatement::CallSubquery(call) => bind_inline_call(ctx, call),
    }
}

fn bind_inline_call(
    ctx: &mut BindContext,
    call: &mut InlineProcedureCall,
) -> Result<(), AnalysisError> {
    if call.variable_scope.is_some() {
        return Err(AnalysisError::NotImplemented {
            message: "explicit variable-scope CALL subqueries require unsupported GP03".into(),
            span: call.span,
            hint: None,
        });
    }
    if call.in_transactions {
        return Err(AnalysisError::NotImplemented {
            message: "CALL { ... } IN TRANSACTIONS is not supported in v1.1".into(),
            span: call.span,
            hint: None,
        });
    }
    expr::check_query_subquery_depth(&call.body, 1)?;
    let bind_result = ctx.with_child_scope(ScopeKind::Subquery, call.span, false, |ctx| {
        bind_query_pipeline(ctx, &mut call.body)
    });
    if let Err(AnalysisError::MutatingProcedureInReadPipeline { span, .. }) = bind_result {
        return Err(AnalysisError::NotImplemented {
            message: "write operations inside CALL { ... } are not supported in v1.1".into(),
            span,
            hint: None,
        });
    }
    bind_result?;
    let outputs = inline_call_outputs(ctx, &call.body)?;
    declare_inline_call_yields(ctx, call, &outputs)
}

#[derive(Clone)]
struct InlineCallOutput {
    name: selene_core::IStr,
    ty: AnalyzedType,
    span: crate::SourceSpan,
}

fn inline_call_outputs(
    ctx: &BindContext,
    body: &QueryPipeline,
) -> Result<Vec<InlineCallOutput>, AnalysisError> {
    let Some(return_clause) = body.statements.iter().rev().find_map(|statement| {
        if let PipelineStatement::Return(clause) = statement {
            Some(clause)
        } else {
            None
        }
    }) else {
        return Ok(Vec::new());
    };
    if return_clause.star {
        return Ok(Vec::new());
    }
    Ok(return_clause
        .items
        .iter()
        .filter_map(|item| {
            projection_name(item).map(|name| {
                let ty = ctx
                    .expr_id(&item.expr)
                    .map(|id| ctx.expr_type(id).clone())
                    .unwrap_or(AnalyzedType::Dynamic);
                InlineCallOutput {
                    name,
                    ty,
                    span: item.span,
                }
            })
        })
        .collect::<Vec<_>>())
}

fn declare_inline_call_yields(
    ctx: &mut BindContext,
    call: &InlineProcedureCall,
    outputs: &[InlineCallOutput],
) -> Result<(), AnalysisError> {
    if call.yield_items.is_empty() {
        return Ok(());
    }
    for item in &call.yield_items {
        match item.column {
            YieldColumn::Star => {
                for output in outputs {
                    ctx.declare_strict_typed(
                        BindingDeclKind::YieldColumn,
                        output.name,
                        item.span,
                        output.ty.clone(),
                    )?;
                }
            }
            YieldColumn::Named(column) => {
                let Some(output) = outputs.iter().find(|output| output.name == column) else {
                    return Err(AnalysisError::UnknownYieldColumn {
                        procedure: Box::new([]),
                        column,
                        span: item.span,
                    });
                };
                ctx.declare_strict_typed(
                    BindingDeclKind::YieldColumn,
                    item.alias.unwrap_or(column),
                    output.span,
                    output.ty.clone(),
                )?;
            }
        }
    }
    Ok(())
}

fn projection_name(item: &ReturnItem) -> Option<selene_core::IStr> {
    item.alias.or({
        if let ValueExpr::Variable { name, .. } = &item.expr {
            Some(*name)
        } else {
            None
        }
    })
}

pub(crate) fn bind_return_clause(
    ctx: &mut BindContext,
    clause: &ReturnClause,
) -> Result<(), AnalysisError> {
    bind_return_inputs(
        ctx,
        &clause.items,
        clause.group_by.as_deref(),
        clause.having.as_ref(),
    )?;
    // Non-boundary projection scope: ISO GA07 lets ORDER BY / OFFSET /
    // LIMIT reach the pre-RETURN bindings, and `RETURN *` keeps the whole
    // input row visible by virtue of the parent walk.
    ctx.enter_projection_scope(clause.span, false);
    declare_projection_items(ctx, &clause.items)
}

fn bind_with_clause(ctx: &mut BindContext, clause: &WithClause) -> Result<(), AnalysisError> {
    bind_return_inputs(
        ctx,
        &clause.items,
        clause.group_by.as_deref(),
        clause.having.as_ref(),
    )?;
    // Boundary projection scope: pre-WITH bindings end here. Post-WITH
    // clauses see only the projection aliases declared below.
    ctx.enter_projection_scope(clause.span, true);
    declare_projection_items(ctx, &clause.items)?;
    if let Some(where_clause) = &clause.where_clause {
        expr::bind_condition(ctx, where_clause, ConditionClause::WithWhere)?;
    }
    Ok(())
}

fn bind_return_inputs(
    ctx: &mut BindContext,
    items: &[ReturnItem],
    group_by: Option<&[ValueExpr]>,
    having: Option<&ValueExpr>,
) -> Result<(), AnalysisError> {
    for item in items {
        expr::bind_value_expr(ctx, &item.expr)?;
    }
    if let Some(values) = group_by {
        for value in values {
            expr::bind_value_expr(ctx, value)?;
        }
    }
    if let Some(value) = having {
        expr::bind_condition(ctx, value, ConditionClause::Having)?;
    }
    Ok(())
}

fn declare_projection_items(
    ctx: &mut BindContext,
    items: &[ReturnItem],
) -> Result<(), AnalysisError> {
    for item in items {
        let ty = projection_item_type(ctx, item);
        if let Some(alias) = item.alias {
            ctx.declare_strict_typed(BindingDeclKind::ProjectionAlias, alias, item.span, ty)?;
        } else if let ValueExpr::Variable { name, span } = &item.expr {
            ctx.declare_strict_typed(BindingDeclKind::ProjectionAlias, *name, *span, ty)?;
        }
    }
    Ok(())
}

fn bind_let(ctx: &mut BindContext, bindings: &[LetBinding]) -> Result<(), AnalysisError> {
    for binding in bindings {
        let id = expr::bind_value_expr(ctx, &binding.value)?;
        let ty = ctx.expr_type(id).clone();
        ctx.declare_strict_typed(BindingDeclKind::LetAlias, binding.alias, binding.span, ty)?;
    }
    Ok(())
}

fn bind_unwind(ctx: &mut BindContext, unwind: &UnwindStatement) -> Result<(), AnalysisError> {
    let id = expr::bind_value_expr(ctx, &unwind.source)?;
    let ty = match ctx.expr_type(id) {
        AnalyzedType::Resolved(crate::GqlType::List(inner)) => {
            AnalyzedType::Resolved((**inner).clone())
        }
        _ => AnalyzedType::Dynamic,
    };
    ctx.declare_strict_typed(BindingDeclKind::UnwindAlias, unwind.alias, unwind.span, ty)?;
    Ok(())
}

fn bind_sorting(ctx: &mut BindContext, terms: &[OrderTerm]) -> Result<(), AnalysisError> {
    for term in terms {
        expr::bind_value_expr(ctx, &term.expr)?;
    }
    Ok(())
}

fn bind_limit_value(value: &LimitValue) {
    match value {
        LimitValue::Count(..) | LimitValue::Parameter(..) => {}
    }
}

fn projection_item_type(ctx: &BindContext, item: &ReturnItem) -> AnalyzedType {
    ctx.expr_id(&item.expr)
        .map(|id| ctx.expr_type(id).clone())
        .unwrap_or(AnalyzedType::Dynamic)
}
