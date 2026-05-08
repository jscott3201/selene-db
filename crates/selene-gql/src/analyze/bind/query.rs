//! Read-query bind handling.

use crate::{
    LetBinding, LimitValue, OrderTerm, PipelineStatement, QueryPipeline, ReturnClause, ReturnItem,
    UnwindStatement, ValueExpr, WithClause,
    analyze::{
        binding::BindingDeclKind,
        error::{AnalysisError, ConditionClause},
        types::AnalyzedType,
    },
};

use super::{BindContext, call, expr, pattern};

pub(crate) fn bind_query_pipeline(
    ctx: &mut BindContext,
    pipeline: &QueryPipeline,
) -> Result<(), AnalysisError> {
    for statement in &pipeline.statements {
        bind_pipeline_statement(ctx, statement)?;
    }
    Ok(())
}

pub(crate) fn bind_pipeline_statement(
    ctx: &mut BindContext,
    statement: &PipelineStatement,
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
            call::bind_procedure_call(ctx, call)?;
            Ok(())
        }
    }
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
