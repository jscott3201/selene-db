//! Mutation-pipeline bind handling.

use crate::{
    DeleteStatement, MutationPipeline, MutationStatement, MutationTerminator, RemoveItem, SetItem,
    analyze::{
        binding::BindingUseKind,
        error::{AnalysisError, ConditionClause},
    },
};

use super::{BindContext, expr, pattern, query};

pub(crate) fn bind_mutation_pipeline(
    ctx: &mut BindContext,
    pipeline: &MutationPipeline,
) -> Result<(), AnalysisError> {
    for statement in &pipeline.statements {
        bind_mutation_statement(ctx, statement)?;
    }
    if let Some(terminator) = &pipeline.terminator {
        match terminator {
            MutationTerminator::Return(clause) => query::bind_return_clause(ctx, clause)?,
            MutationTerminator::Finish(_) => {}
        }
    }
    Ok(())
}

fn bind_mutation_statement(
    ctx: &mut BindContext,
    statement: &MutationStatement,
) -> Result<(), AnalysisError> {
    match statement {
        MutationStatement::Match(clause) => pattern::bind_match_clause(ctx, clause),
        MutationStatement::Filter(value) => {
            expr::bind_condition(ctx, value, ConditionClause::Filter)?;
            Ok(())
        }
        MutationStatement::Insert(insert) => {
            for graph_pattern in &insert.patterns {
                pattern::bind_insert_graph_pattern(ctx, graph_pattern)?;
            }
            Ok(())
        }
        MutationStatement::Set(items) => bind_set_items(ctx, items),
        MutationStatement::Remove(items) => bind_remove_items(ctx, items),
        MutationStatement::Delete(statement) => bind_delete(ctx, statement),
    }
}

fn bind_set_items(ctx: &mut BindContext, items: &[SetItem]) -> Result<(), AnalysisError> {
    for item in items {
        match item {
            SetItem::Property {
                target,
                value,
                span,
                ..
            } => {
                ctx.resolve(*target, *span, BindingUseKind::SetTarget)?;
                expr::bind_value_expr(ctx, value)?;
            }
            SetItem::PropertyMerge {
                target,
                properties,
                span,
            } => {
                ctx.resolve(*target, *span, BindingUseKind::SetTarget)?;
                for (_, value) in properties {
                    expr::bind_value_expr(ctx, value)?;
                }
            }
            SetItem::Label { target, span, .. } => {
                ctx.resolve(*target, *span, BindingUseKind::SetTarget)?;
            }
        }
    }
    Ok(())
}

fn bind_remove_items(ctx: &mut BindContext, items: &[RemoveItem]) -> Result<(), AnalysisError> {
    for item in items {
        match item {
            RemoveItem::Property { target, span, .. } | RemoveItem::Label { target, span, .. } => {
                ctx.resolve(*target, *span, BindingUseKind::RemoveTarget)?;
            }
        }
    }
    Ok(())
}

fn bind_delete(ctx: &mut BindContext, statement: &DeleteStatement) -> Result<(), AnalysisError> {
    for item in &statement.items {
        ctx.resolve(*item, statement.span, BindingUseKind::DeleteTarget)?;
    }
    Ok(())
}
