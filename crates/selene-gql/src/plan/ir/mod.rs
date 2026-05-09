//! Planner IR definitions.

mod access;
mod call;
mod catalog;
mod filter;
mod mutation;
mod tx;

use crate::{
    EdgeDirection, LabelExpr, SetOp, SourceSpan,
    analyze::{AnalyzedType, BindingId, ExprId},
};

pub use access::{NodeIdOrdering, OrderAccess, ScanAccess, TypedIndexBounds};
pub use call::{PlannedCall, PlannedYieldItem, YieldKind};
pub use catalog::{CatalogOp, PlannedTypePropertyConstraint, PlannedTypePropertyDef};
pub use filter::{
    Aggregate, AggregateArg, FilterPredicate, FilterPredicateKind, LimitAmount, OrderKey,
    ProjectExpr,
};
pub use mutation::{InsertEndpointRef, InsertSiteId, MutationOp, PropertyInit};
pub use tx::TxOp;

/// Literal execution plan produced by planner lowering.
#[derive(Clone, Debug)]
pub struct ExecutionPlan {
    /// Optional leading pattern plan for query pipelines beginning with MATCH.
    pub pattern_plan: Option<PatternPlan>,
    /// Binding-table operations after the leading pattern phase.
    pub pipeline: Vec<PipelineOp>,
    /// Columns exposed by the final pipeline boundary.
    pub output_schema: BindingTableSchema,
    /// Planner implementation-defined limits.
    pub impl_defined_caps: ImplDefinedCaps,
    /// Next optimizer-owned expression ID for this plan.
    pub next_expr_id: ExprId,
}

impl ExecutionPlan {
    /// Allocate a fresh expression ID for optimizer-synthesized expressions.
    pub(crate) fn alloc_expr_id(&mut self) -> ExprId {
        let id = self.next_expr_id;
        self.next_expr_id = ExprId::new(id.get().saturating_add(1));
        id
    }
}

/// Pattern-matching subplan for the leading MATCH prefix.
#[derive(Clone, Debug)]
pub struct PatternPlan {
    /// Named pattern bindings visible to downstream pipeline operations.
    pub bindings: Vec<BindingDef>,
    /// Unoptimized join tree.
    pub join_tree: JoinTree,
    /// Inline and clause-level predicates attached to the pattern phase.
    pub filters: Vec<FilterPredicate>,
    /// Path-binding placeholders carried for later path execution work.
    pub paths: Vec<PathPlan>,
}

/// Named binding defined by pattern analysis.
#[derive(Clone, Debug, PartialEq)]
pub struct BindingDef {
    /// Analyzer-stable binding ID.
    pub binding: BindingId,
    /// Interned binding name.
    pub name: selene_core::IStr,
    /// Element kind represented by the binding.
    pub element: BindingElement,
    /// Analyzer-inferred binding type.
    pub ty: AnalyzedType,
    /// Static label predicate from the declaring pattern, when present.
    pub label_predicate: Option<LabelExpr>,
    /// Source span of the declaration.
    pub span: SourceSpan,
}

/// Binding element category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingElement {
    /// Node binding.
    Node,
    /// Edge binding.
    Edge,
    /// Path binding.
    Path,
    /// Value alias binding.
    Alias,
}

/// Pattern join tree.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum JoinTree {
    /// Scan nodes or edges.
    Scan(NodeOrEdgeScan),
    /// Expand from a child tree across one edge pattern.
    Expand {
        /// Input side of the expansion.
        child: Box<JoinTree>,
        /// Edge pattern to traverse.
        edge: EdgeMatch,
        /// Direction requested by the source pattern.
        direction: EdgeDirection,
    },
    /// Binary join between two pattern fragments.
    HashJoin {
        /// Left input.
        left: Box<JoinTree>,
        /// Right input.
        right: Box<JoinTree>,
        /// Shared binding names used as the join key.
        key: Vec<selene_core::IStr>,
        /// Planner-selected build input.
        build_side: BuildSide,
    },
    /// Left-outer join used for OPTIONAL MATCH.
    Outer {
        /// Preserved left input.
        left: Box<JoinTree>,
        /// Optional right input.
        right: Box<JoinTree>,
        /// Shared binding names used as the join key.
        key: Vec<selene_core::IStr>,
    },
    /// Marker for future WCO rewrites.
    WorstCaseOptimal {
        /// Intersected subplans.
        intersection: Vec<JoinTree>,
        /// Node-id orderings used to break symmetric WCO traversals.
        node_id_ordering: Vec<NodeIdOrdering>,
    },
    /// Nested subplan placeholder.
    Subplan(Box<ExecutionPlan>),
}

/// Planner-selected hash-join build side.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildSide {
    /// Build the hash table from the left input.
    Left,
    /// Build the hash table from the right input.
    Right,
}

/// Node or edge scan.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeOrEdgeScan {
    /// Named binding, or `None` for anonymous pattern elements.
    pub binding: Option<BindingId>,
    /// Scan kind.
    pub kind: ScanKind,
    /// Label predicate attached to the scanned element.
    pub label_predicate: Option<LabelExpr>,
    /// Inline property predicates from the pattern.
    pub property_predicates: Vec<FilterPredicate>,
    /// Optimizer-selected access path.
    pub access: ScanAccess,
    /// Source span.
    pub span: SourceSpan,
}

/// Scan element kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanKind {
    /// Node scan.
    Node,
    /// Edge scan.
    Edge,
}

/// Edge pattern in an expansion.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeMatch {
    /// Named edge binding, or `None` for anonymous edge patterns.
    pub binding: Option<BindingId>,
    /// Label predicate attached to the edge.
    pub label_predicate: Option<LabelExpr>,
    /// Inline property predicates from the edge pattern.
    pub property_predicates: Vec<FilterPredicate>,
    /// Binding on the syntactic left side of the edge, if named.
    pub left_binding: Option<BindingId>,
    /// Binding on the syntactic right side of the edge, if named.
    pub right_binding: Option<BindingId>,
    /// Label predicate on the syntactic right-side node, if any.
    pub right_label_predicate: Option<LabelExpr>,
    /// Property-map equality predicates on the syntactic right-side node.
    pub right_property_predicates: Vec<FilterPredicate>,
    /// Optimizer-selected access path.
    pub access: ScanAccess,
    /// Source span.
    pub span: SourceSpan,
}

/// Pipeline operation over binding tables.
///
/// `#[non_exhaustive]` so future planner work (e.g., MERGE lowering, CALL
/// subquery form, INDEX DDL via selene-pack) can add variants without
/// breaking downstream pattern matches.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum PipelineOp {
    /// Retain rows satisfying a predicate.
    Filter(FilterPredicate),
    /// Project expressions into output columns.
    Project(Vec<ProjectExpr>),
    /// Extend the binding table with new aliases without dropping prior columns.
    Let(Vec<ProjectExpr>),
    /// Expand a list expression to one row per element.
    Unwind {
        /// Source list expression.
        source: ProjectExpr,
        /// Alias bound to each list element.
        alias: selene_core::IStr,
        /// Source span.
        span: SourceSpan,
    },
    /// Sort rows.
    OrderBy(Vec<OrderKey>),
    /// Offset and limit rows.
    Limit {
        /// Rows to skip.
        offset: LimitAmount,
        /// Rows to retain after offset.
        count: LimitAmount,
    },
    /// Sort rows while retaining only the bounded top range.
    TopK {
        /// Sort keys preserved from the fused `OrderBy`.
        keys: Vec<OrderKey>,
        /// Rows to skip before yielding.
        offset: LimitAmount,
        /// Rows to retain after offset.
        count: LimitAmount,
    },
    /// Group and aggregate rows.
    GroupBy {
        /// Grouping keys.
        keys: Vec<ProjectExpr>,
        /// Aggregate expressions.
        aggregates: Vec<Aggregate>,
    },
    /// Deduplicate rows.
    Distinct,
    /// Apply a set-composition operation with another plan.
    Union {
        /// Parser set operator, preserved exactly.
        op: SetOp,
        /// Right-hand plan.
        rhs: Box<ExecutionPlan>,
    },
    /// Evaluate a NEXT block after the current plan.
    Chain(Box<ExecutionPlan>),
    /// Planned procedure call.
    Call(PlannedCall),
    /// Mutation operation.
    Mutation(MutationOp),
    /// Catalog operation.
    Catalog(CatalogOp),
    /// Transaction-control operation.
    Tx(TxOp),
}

/// Planner implementation-defined limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ImplDefinedCaps {
    /// Maximum accepted variable-length quantifier upper bound.
    pub max_quantifier: u32,
    /// Fixed-point optimizer iteration cap.
    pub max_optimizer_iterations: u32,
    /// Default maximum path length for future path execution.
    pub max_path_length: u32,
    /// Maximum number of expand nodes WCO cycle detection will inspect.
    pub max_wco_traversal_nodes: u32,
}

impl Default for ImplDefinedCaps {
    fn default() -> Self {
        Self {
            max_quantifier: 100,
            max_optimizer_iterations: 8,
            max_path_length: 32,
            max_wco_traversal_nodes: 64,
        }
    }
}

/// Binding-table output schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingTableSchema {
    /// Output columns in order.
    pub columns: Vec<BindingTableColumn>,
}

/// One binding-table output column.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingTableColumn {
    /// Stable column name for aliases and bare variable projections.
    pub name: Option<selene_core::IStr>,
    /// Analyzer-inferred column type.
    pub ty: AnalyzedType,
}

/// Path binding placeholder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathPlan {
    /// Analyzer path binding.
    pub binding: BindingId,
    /// Source span.
    pub span: SourceSpan,
}
