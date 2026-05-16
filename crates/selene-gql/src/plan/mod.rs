//! Query planner entry points and execution-plan IR.
//!
//! The planner lowers a [`crate::AnalyzedStatement`] into a literal
//! [`ExecutionPlan`] covering reads, mutations, DDL, CALL, and transaction
//! control, with binding-table schemas attached wherever row shape changes.
//! Optimizer rewrites are explicit: callers that want canonicalization or
//! access-path selection call [`optimize()`] after [`plan()`]. This layer relies
//! on analyzer binding/type/write-set invariants and defers provider fanout,
//! three-valued logic evaluation, and transaction effects to the runtime. See
//! Spec 08 §6-§8 and Spec 13.

mod error;
mod ir;
mod lowering;
pub mod optimize;

pub use error::PlannerError;
pub use ir::{
    Aggregate, AggregateArg, BindingDef, BindingElement, BindingTableColumn, BindingTableSchema,
    BuildSide, CatalogOp, EdgeMatch, ExecutionPlan, FilterPredicate, FilterPredicateKind,
    HiddenBindingId, ImplDefinedCaps, InsertEndpointRef, InsertSiteId, JoinTree, LimitAmount,
    MutationOp, NodeIdOrdering, NodeOrEdgeScan, OrderAccess, OrderKey, PathPlan, PatternPlan,
    PipelineOp, PipelineOpId, PlannedCall, PlannedTypePropertyConstraint, PlannedTypePropertyDef,
    PlannedYieldItem, ProjectExpr, PropertyInit, ScanAccess, ScanKind, TxOp, TypedIndexBounds,
    YieldKind,
};
pub use lowering::plan;
pub use optimize::{
    CompositeIndexHandle, EdgeStatistics, EmptyIndexCatalog, IndexCatalog, IndexHandle, IndexKind,
    IndexTarget, OptimizeContext, PropertyHistogram, Rule, Transformed, TypedIndexLookup,
    WanderJoinSampler, optimize,
};
