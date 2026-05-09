//! Query planner entry points and unoptimized execution-plan IR.
//!
//! Lowers an [`AnalyzedStatement`] into a literal, unoptimized
//! [`ExecutionPlan`] covering reads, mutations, DDL, CALL, and
//! transaction control. The optimizer is explicit: callers that want rewrites
//! call [`optimize`] after [`plan`]. The durable planner contract is spec 13;
//! `_spec/13-iso-gql-planner.md` is a
//! local-only mirror per the underscore-folder convention.

mod error;
mod ir;
mod lowering;
pub mod optimize;

pub use error::PlannerError;
pub use ir::{
    Aggregate, AggregateArg, BindingDef, BindingElement, BindingTableColumn, BindingTableSchema,
    CatalogOp, EdgeMatch, ExecutionPlan, FilterPredicate, FilterPredicateKind, ImplDefinedCaps,
    InsertEndpointRef, InsertSiteId, JoinTree, LimitAmount, MutationOp, NodeOrEdgeScan, OrderKey,
    PathPlan, PatternPlan, PipelineOp, PlannedCall, PlannedTypePropertyConstraint,
    PlannedTypePropertyDef, PlannedYieldItem, ProjectExpr, PropertyInit, ScanKind, TxOp, YieldKind,
};
pub use lowering::plan;
pub use optimize::{
    EdgeStatistics, OptimizeContext, PropertyHistogram, Rule, Transformed, WanderJoinSampler,
    optimize,
};
