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
    BuildSide, CatalogOp, EdgeMatch, ExecutionPlan, FilterPredicate, FilterPredicateKind,
    HiddenBindingId, ImplDefinedCaps, InsertEndpointRef, InsertSiteId, JoinTree, LimitAmount,
    MutationOp, NodeIdOrdering, NodeOrEdgeScan, OrderAccess, OrderKey, PathPlan, PatternPlan,
    PipelineOp, PlannedCall, PlannedTypePropertyConstraint, PlannedTypePropertyDef,
    PlannedYieldItem, ProjectExpr, PropertyInit, ScanAccess, ScanKind, TxOp, TypedIndexBounds,
    YieldKind,
};
pub use lowering::plan;
pub use optimize::{
    CompositeIndexHandle, EdgeStatistics, EmptyIndexCatalog, IndexCatalog, IndexHandle, IndexKind,
    IndexTarget, OptimizeContext, PropertyHistogram, Rule, Transformed, TypedIndexLookup,
    WanderJoinSampler, optimize,
};
