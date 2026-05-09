//! Query planner entry points and unoptimized execution-plan IR.
//!
//! BRIEF-26 introduces the first planner phase: lower an
//! [`AnalyzedStatement`] into a literal, unoptimized [`ExecutionPlan`].
//! Optimizer rules and write-side lowering land in later M5c briefs. The
//! durable planner contract is spec 13; `_spec/13-iso-gql-planner.md` is a
//! local-only mirror per the underscore-folder convention.

mod error;
mod ir;
mod lowering;

pub use error::PlannerError;
pub use ir::{
    Aggregate, AggregateArg, BindingDef, BindingElement, BindingTableColumn, BindingTableSchema,
    EdgeMatch, ExecutionPlan, FilterPredicate, FilterPredicateKind, ImplDefinedCaps, JoinTree,
    LimitAmount, MutationOp, NodeOrEdgeScan, OrderKey, PathPlan, PatternPlan, PipelineOp,
    PlannedCall, PlannedYieldItem, ProjectExpr, ScanKind,
};
pub use lowering::plan;
