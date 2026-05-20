//! Binding-table pipeline executor.

mod aggregate;
mod call;
mod catalog;
mod chain;
mod distinct;
mod filter;
mod group_by;
mod let_op;
mod limit;
mod mutation;
mod order_by;
mod project;
mod top_k;
pub(crate) mod tx;
mod union;
mod unwind;

use crate::{
    PipelineOp, SubqueryRegistry,
    analyze::ExprIdLookup,
    runtime::{BindingTable, EvalCtx, ExecutorError, TxContext},
};

/// Execute a sequence of pipeline operations against a binding table.
pub fn execute_pipeline(
    pipeline: &[PipelineOp],
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let expr_ids = ExprIdLookup::default();
    let subqueries = SubqueryRegistry::default();
    let (expr_ids, subqueries) = ctx.plan_metadata().unwrap_or((&expr_ids, &subqueries));
    execute_pipeline_with_plan(pipeline, table, ctx, expr_ids, subqueries)
}

pub(crate) fn execute_pipeline_with_plan(
    pipeline: &[PipelineOp],
    mut table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
    expr_ids: &ExprIdLookup,
    subqueries: &SubqueryRegistry,
) -> Result<BindingTable, ExecutorError> {
    for op in pipeline {
        table = match op {
            PipelineOp::Filter(predicate) => {
                let eval_ctx = EvalCtx {
                    tx: ctx,
                    expr_ids,
                    subqueries,
                };
                filter::execute(predicate, table, &eval_ctx)?
            }
            PipelineOp::Project(items) => {
                let eval_ctx = EvalCtx {
                    tx: ctx,
                    expr_ids,
                    subqueries,
                };
                project::execute(items, table, &eval_ctx)?
            }
            PipelineOp::Let(items) => {
                let eval_ctx = EvalCtx {
                    tx: ctx,
                    expr_ids,
                    subqueries,
                };
                let_op::execute(items, table, &eval_ctx)?
            }
            PipelineOp::Unwind {
                source,
                alias,
                span,
            } => {
                let eval_ctx = EvalCtx {
                    tx: ctx,
                    expr_ids,
                    subqueries,
                };
                unwind::execute(source, *alias, *span, table, &eval_ctx)?
            }
            PipelineOp::OrderBy(keys) => {
                let eval_ctx = EvalCtx {
                    tx: ctx,
                    expr_ids,
                    subqueries,
                };
                order_by::execute(keys, table, &eval_ctx)?
            }
            PipelineOp::Limit { offset, count } => limit::execute(offset, count, table, ctx)?,
            PipelineOp::TopK {
                keys,
                offset,
                count,
            } => {
                let eval_ctx = EvalCtx {
                    tx: ctx,
                    expr_ids,
                    subqueries,
                };
                top_k::execute(keys, offset, count, table, ctx, &eval_ctx)?
            }
            PipelineOp::GroupBy { keys, aggregates } => {
                let eval_ctx = EvalCtx {
                    tx: ctx,
                    expr_ids,
                    subqueries,
                };
                group_by::execute(keys, aggregates, table, &eval_ctx)?
            }
            PipelineOp::Distinct => distinct::execute(table),
            PipelineOp::Union { op, rhs } => union::execute(*op, rhs, table, ctx)?,
            PipelineOp::Chain(rhs) => chain::execute(rhs, table, ctx)?,
            PipelineOp::Mutation(mutation) => {
                mutation::execute(mutation, table, ctx, expr_ids, subqueries)?
            }
            PipelineOp::Catalog(catalog) => catalog::execute(catalog, table, ctx)?,
            PipelineOp::Call(call) => call::execute(call, table, ctx, expr_ids, subqueries)?,
            PipelineOp::Tx(_) => {
                return Err(ExecutorError::ImplementationDefined {
                    detail: "TX op surfaced inside execute_pipeline; should be dispatched at statement level",
                });
            }
        };
    }
    Ok(table)
}
