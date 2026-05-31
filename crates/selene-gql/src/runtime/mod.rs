//! GQL execution runtime.
//!
//! The runtime consumes an optimized `ExecutionPlan` and a transaction context,
//! walks pattern join trees into binding tables, applies pipeline operators,
//! dispatches procedure calls through tier-checked contexts, and coordinates
//! statement-level transaction control. It relies on parser, analyzer, and
//! planner invariants for binding and type structure; this layer owns runtime
//! three-valued logic, mutation routing through `Mutator`, provider error
//! propagation, and statement output shaping. See Spec 08 §5-§8 and Spec 14
//! §3-§8.

mod binding_table;
mod binding_table_registry;
mod builtin_registry;
mod builtins;
mod call_plan_cache;
mod context;
mod context_tiers;
mod error;
pub(crate) mod evaluator;
mod expand;
mod hash_join;
mod native_algorithms;
mod outer;
mod parameter_type;
mod path_mode;
mod path_search;
mod pattern;
mod pipeline;
mod plan_cache;
mod plan_runner;
mod questioned;
mod repeat;
mod scan;
mod scan_resolve;
mod session;
#[cfg(any(test, feature = "test-harness"))]
mod snapshot_summary;
mod statement;
mod subplan;
mod value_compare;
mod value_key;
mod value_type_match;
mod visited_set;
mod wco;

pub use binding_table::{Binding, BindingTable};
pub use binding_table_registry::BindingTableRegistry;
pub use builtin_registry::BuiltinProcedureRegistry;
pub use call_plan_cache::{CallPlanCache, CallPlanCacheStats, CallPlanKey};
pub use context::{AdaptiveOptimizer, EvalCtx, TxContext};
pub use context_tiers::{GraphContext, MutationContext, ProcedureContext};
pub use error::{DataExceptionSubclass, ExecutorError, ExecutorWarning, WarningSink};
pub use pattern::execute_pattern;
pub use pipeline::execute_pipeline;
pub use plan_cache::{PlanCache, PlanCacheStats};
pub(crate) use plan_runner::execute_plan;
pub use session::{RollbackOutcome, Session, SessionParameterValue, TransactionOutcome};
#[cfg(any(test, feature = "test-harness"))]
pub use snapshot_summary::{
    ExecutorSnapshot, ExecutorSummaryInput, NetGraphDelta, RowOrderPolicy, SnapshotColumn,
    executor_summary,
};
pub use statement::{StatementOutput, WriteOutcome, execute_statement};

pub use crate::plan::{BindingTableColumn, BindingTableSchema};

#[cfg(any(test, feature = "test-harness"))]
pub use evaluator::evaluate_for_test;
