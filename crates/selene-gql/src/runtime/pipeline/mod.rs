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
    PipelineOp,
    runtime::{BindingTable, ExecutorError, TxContext},
};

/// Execute a sequence of pipeline operations against a binding table.
pub fn execute_pipeline(
    pipeline: &[PipelineOp],
    mut table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    for op in pipeline {
        table = match op {
            PipelineOp::Filter(predicate) => filter::execute(predicate, table, ctx)?,
            PipelineOp::Project(items) => project::execute(items, table, ctx)?,
            PipelineOp::Let(items) => let_op::execute(items, table, ctx)?,
            PipelineOp::Unwind {
                source,
                alias,
                span,
            } => unwind::execute(source, *alias, *span, table, ctx)?,
            PipelineOp::OrderBy(keys) => order_by::execute(keys, table, ctx)?,
            PipelineOp::Limit { offset, count } => limit::execute(offset, count, table, ctx)?,
            PipelineOp::TopK {
                keys,
                offset,
                count,
            } => top_k::execute(keys, offset, count, table, ctx)?,
            PipelineOp::GroupBy { keys, aggregates } => {
                group_by::execute(keys, aggregates, table, ctx)?
            }
            PipelineOp::Distinct => distinct::execute(table),
            PipelineOp::Union { op, rhs } => union::execute(*op, rhs, table, ctx)?,
            PipelineOp::Chain(rhs) => chain::execute(rhs, table, ctx)?,
            PipelineOp::Mutation(mutation) => mutation::execute(mutation, table, ctx)?,
            PipelineOp::Catalog(catalog) => catalog::execute(catalog, table, ctx)?,
            PipelineOp::Call(call) => call::execute(call, table, ctx)?,
            PipelineOp::Tx(_) => {
                return Err(ExecutorError::ImplementationDefined {
                    detail: "TX op surfaced inside execute_pipeline; should be dispatched at statement level",
                });
            }
        };
    }
    Ok(table)
}
