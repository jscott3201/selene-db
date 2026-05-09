//! Query executor runtime surfaces.
//!
//! BRIEF-31 intentionally exposes only the scan/evaluator scaffolding needed
//! for M5d Phase A. Later executor briefs add join, pipeline, mutation,
//! catalog, transaction-control, and CALL dispatch.

mod binding_table;
mod context;
mod error;
pub(crate) mod evaluator;
mod scan;
mod value_compare;

pub use binding_table::{Binding, BindingTable};
pub use context::TxContext;
pub use error::ExecutorError;
pub use scan::scan_pattern;

pub use crate::plan::{BindingTableColumn, BindingTableSchema};

#[cfg(any(test, feature = "test-harness"))]
pub use evaluator::evaluate as evaluate_for_test;
