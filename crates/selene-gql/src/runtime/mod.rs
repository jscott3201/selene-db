//! Query executor runtime surfaces.
//!
//! BRIEF-31 intentionally exposes only the scan/evaluator scaffolding needed
//! for M5d Phase A. Later executor briefs add join, pipeline, mutation,
//! catalog, transaction-control, and CALL dispatch.

mod binding_table;
mod context;
mod context_tiers;
mod error;
pub(crate) mod evaluator;
mod expand;
mod hash_join;
mod outer;
mod pattern;
mod pipeline;
mod plan_runner;
mod scan;
mod session;
#[cfg(any(test, feature = "test-harness"))]
mod snapshot_summary;
mod statement;
mod subplan;
mod value_compare;
mod value_key;
mod wco;

pub use binding_table::{Binding, BindingTable};
pub use context::{AdaptiveOptimizer, TxContext};
pub use context_tiers::{GraphContext, MutationContext, ProcedureContext};
pub use error::ExecutorError;
pub use pattern::execute_pattern;
pub use pipeline::execute_pipeline;
pub(crate) use plan_runner::execute_plan;
pub use session::Session;
#[cfg(any(test, feature = "test-harness"))]
pub use snapshot_summary::{
    ExecutorSnapshot, ExecutorSummaryInput, NetGraphDelta, RowOrderPolicy, SnapshotColumn,
    executor_summary,
};
pub use statement::{StatementOutput, execute_statement};

pub use crate::plan::{BindingTableColumn, BindingTableSchema};

#[cfg(any(test, feature = "test-harness"))]
pub use evaluator::evaluate as evaluate_for_test;
