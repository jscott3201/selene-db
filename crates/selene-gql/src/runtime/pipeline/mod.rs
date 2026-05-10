//! Binding-table pipeline executor.

mod aggregate;
mod distinct;
mod filter;
mod group_by;
mod let_op;
mod limit;
mod order_by;
mod project;
mod top_k;
mod unwind;

use crate::{
    PipelineOp,
    runtime::{BindingTable, ExecutorError, TxContext},
};

/// Execute a sequence of pipeline operations against a binding table.
pub fn execute_pipeline(
    pipeline: &[PipelineOp],
    mut table: BindingTable,
    ctx: &TxContext<'_>,
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
            PipelineOp::Limit { offset, count } => limit::execute(offset, count, table)?,
            PipelineOp::TopK {
                keys,
                offset,
                count,
            } => top_k::execute(keys, offset, count, table, ctx)?,
            PipelineOp::GroupBy { keys, aggregates } => {
                group_by::execute(keys, aggregates, table, ctx)?
            }
            PipelineOp::Distinct => distinct::execute(table),
            PipelineOp::Union { .. }
            | PipelineOp::Chain(_)
            | PipelineOp::Call(_)
            | PipelineOp::Mutation(_)
            | PipelineOp::Catalog(_)
            | PipelineOp::Tx(_) => {
                return Err(ExecutorError::ImplementationDefined {
                    detail: "pipeline op not implemented",
                });
            }
        };
    }
    Ok(table)
}
