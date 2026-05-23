//! Planner IR definitions.

mod access;
mod call;
mod catalog;
mod filter;
mod mutation;
mod subquery;
mod tx;

use std::num::NonZeroUsize;

use crate::{
    EdgeDirection, LabelExpr, PathMode, PathSelector, SetOp, SourceSpan,
    analyze::{AnalyzedType, BindingId, ExprId, ExprIdLookup, StatementCategory},
};

pub use access::{NodeIdOrdering, OrderAccess, ScanAccess, TypedIndexBounds};
pub use call::{PlannedCall, PlannedYieldItem, YieldKind};
pub use catalog::{CatalogOp, PlannedTypePropertyConstraint, PlannedTypePropertyDef};
pub use filter::{
    Aggregate, AggregateArg, FilterPredicate, FilterPredicateKind, LimitAmount, OrderKey,
    ProjectExpr,
};
pub use mutation::{InsertEndpointRef, InsertSiteId, MutationOp, PropertyInit};
pub use subquery::{
    OuterBindingRef, PlannedSubquery, PlannedTableSubquery, PlannedTableSubqueryYield,
    SubqueryBody, SubqueryKind, SubqueryRegistry,
};
pub use tx::TxOp;

/// Identifier for a pipeline op within one execution plan.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct PipelineOpId(u32);

impl PipelineOpId {
    pub(crate) const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Return the zero-based op identifier.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Literal execution plan produced by planner lowering.
#[derive(Clone, Debug)]
pub struct ExecutionPlan {
    /// Analyzer-derived statement category used by top-level execution.
    pub category: StatementCategory,
    /// Optional leading pattern plan for query pipelines beginning with MATCH.
    pub pattern_plan: Option<PatternPlan>,
    /// Binding-table operations after the leading pattern phase.
    pub pipeline: Vec<PipelineOp>,
    /// Columns exposed by the final pipeline boundary.
    pub output_schema: BindingTableSchema,
    /// Planner implementation-defined limits.
    pub impl_defined_caps: ImplDefinedCaps,
    /// Analyzer expression lookup cloned into the executable plan.
    pub expr_ids: ExprIdLookup,
    /// Planned expression subqueries indexed by their analyzer expression ID.
    pub subqueries: SubqueryRegistry,
    /// Next optimizer-owned expression ID for this plan.
    pub next_expr_id: ExprId,
    /// Next executor-observable pipeline op ID for this plan.
    pub next_pipeline_op_id: PipelineOpId,
}

impl ExecutionPlan {
    /// Allocate a fresh expression ID for optimizer-synthesized expressions.
    pub(crate) fn alloc_expr_id(&mut self) -> ExprId {
        let id = self.next_expr_id;
        self.next_expr_id = ExprId::new(id.get().saturating_add(1));
        id
    }

    /// Allocate a fresh pipeline op ID for future adaptive execution hooks.
    #[allow(dead_code)]
    pub(crate) fn alloc_pipeline_op_id(&mut self) -> PipelineOpId {
        let id = self.next_pipeline_op_id;
        self.next_pipeline_op_id = PipelineOpId::new(id.get().saturating_add(1));
        id
    }

    /// Recompute the pipeline-op ID high-water mark after lowering or rewrite.
    pub(crate) fn refresh_pipeline_op_high_water(&mut self) {
        if let Some(pattern) = &mut self.pattern_plan {
            refresh_join_tree_pipeline_op_high_water(&mut pattern.join_tree);
        }
        for op in &mut self.pipeline {
            match op {
                PipelineOp::Union { rhs, .. } | PipelineOp::Chain(rhs) => {
                    rhs.refresh_pipeline_op_high_water();
                }
                PipelineOp::CallSubquery(subquery) => {
                    subquery.body.refresh_pipeline_op_high_water();
                }
                PipelineOp::ExplainPlan { inner, .. } => inner.refresh_pipeline_op_high_water(),
                _ => {}
            }
        }
        // Today pipeline ops do not carry stable IDs, so the next ID is len().
        // When ops gain stored IDs, this must scan for max(existing_id) + 1.
        self.next_pipeline_op_id = PipelineOpId::new(self.pipeline.len() as u32);
    }
}

fn refresh_join_tree_pipeline_op_high_water(tree: &mut JoinTree) {
    match tree {
        JoinTree::Scan(_) => {}
        JoinTree::Expand { child, .. }
        | JoinTree::Questioned { child, .. }
        | JoinTree::Repeat { child, .. }
        | JoinTree::PathSearch { child, .. }
        | JoinTree::PathModeFilter { child, .. } => refresh_join_tree_pipeline_op_high_water(child),
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            refresh_join_tree_pipeline_op_high_water(left);
            refresh_join_tree_pipeline_op_high_water(right);
        }
        JoinTree::WorstCaseOptimal { intersection, .. } => {
            for branch in intersection {
                refresh_join_tree_pipeline_op_high_water(branch);
            }
        }
        JoinTree::Subplan(plan) => plan.refresh_pipeline_op_high_water(),
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

/// Binding endpoint used by path-level operators.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TailBinding {
    /// Named pattern binding.
    Named(BindingId),
    /// Executor-private hidden binding.
    Hidden(HiddenBindingId),
}

impl TailBinding {
    /// Return the named binding, if any.
    #[must_use]
    pub const fn named(self) -> Option<BindingId> {
        match self {
            Self::Named(binding) => Some(binding),
            Self::Hidden(_) => None,
        }
    }

    /// Return the hidden binding, if any.
    #[must_use]
    pub const fn hidden(self) -> Option<HiddenBindingId> {
        match self {
            Self::Named(_) => None,
            Self::Hidden(binding) => Some(binding),
        }
    }
}

/// Source of one hop-count contribution for a path-search row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HopContributor {
    /// Static hop count from a fixed-length chain segment.
    Fixed(u32),
    /// One fixed hop from a named edge binding.
    EdgeNamed(BindingId),
    /// One fixed hop from an anonymous edge hidden binding.
    EdgeHidden(HiddenBindingId),
    /// Optional hop from a named questioned edge binding.
    QuestionedNamed(BindingId),
    /// Optional hop from an anonymous questioned edge hidden binding.
    QuestionedHidden(HiddenBindingId),
    /// Runtime hop count from a named quantified-edge group binding.
    GroupNamed(BindingId),
    /// Runtime hop count from an anonymous quantified-edge hidden binding.
    GroupHidden(HiddenBindingId),
}

/// Source of one ordered path-mode validation contribution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathContributor {
    /// Node binding in binding-path order.
    Node(TailBinding),
    /// Fixed edge identity from a named edge binding.
    EdgeNamed(BindingId),
    /// Fixed edge identity from an executor-private edge binding.
    EdgeHidden(HiddenBindingId),
    /// Optional edge identity and final node from a named questioned edge binding.
    QuestionedEdgeNamed {
        /// Runtime binding containing either `EdgeRef` or `NULL`.
        binding: BindingId,
        /// Node binding on the final side when the edge is present.
        final_binding: TailBinding,
    },
    /// Optional edge identity and final node from an anonymous questioned edge binding.
    QuestionedEdgeHidden {
        /// Runtime hidden slot containing either `EdgeRef` or `NULL`.
        hidden: HiddenBindingId,
        /// Node binding on the final side when the edge is present.
        final_binding: TailBinding,
    },
    /// Quantified edge group with enough topology to rebuild intermediate nodes.
    EdgeGroupNamed {
        /// Runtime group binding containing `LIST<EdgeRef>`.
        binding: BindingId,
        /// Node binding at the source side of the quantified segment.
        source: TailBinding,
        /// Direction requested by the quantified edge pattern.
        direction: EdgeDirection,
    },
    /// Anonymous quantified edge group with enough topology to rebuild intermediate nodes.
    EdgeGroupHidden {
        /// Runtime hidden slot containing `LIST<EdgeRef>`.
        hidden: HiddenBindingId,
        /// Node binding at the source side of the quantified segment.
        source: TailBinding,
        /// Direction requested by the quantified edge pattern.
        direction: EdgeDirection,
    },
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
    /// Optional single-edge expansion for ISO questioned path primary (`?`).
    ///
    /// The skipped row binds the edge as `NULL` and unifies the final node
    /// with the source node. The taken row behaves like a one-hop expansion.
    Questioned {
        /// Input side of the expansion.
        child: Box<JoinTree>,
        /// Edge pattern to traverse when the optional edge is present.
        edge: EdgeMatch,
        /// Direction requested by the source pattern.
        direction: EdgeDirection,
        /// Binding for the source-side node.
        source_binding: TailBinding,
        /// Binding for the final node after the questioned edge.
        final_binding: TailBinding,
    },
    /// Bounded or future variable-length expansion across one edge pattern.
    ///
    /// `max: None` represents an ISO unbounded quantifier after the analyzer's
    /// 16.4 legality gate has authorized it.
    Repeat {
        /// Input side of the expansion.
        child: Box<JoinTree>,
        /// Quantified edge pattern to traverse.
        edge: RepeatEdgeMatch,
        /// Direction requested by the source pattern.
        direction: EdgeDirection,
        /// Minimum number of hops.
        min: u32,
        /// Maximum number of hops, or `None` for future unbounded forms.
        max: Option<u32>,
        /// Path mode in scope for this repeat.
        path_mode: PathMode,
        /// Path selector in scope for this repeat.
        selector: Option<PathSelector>,
    },
    /// Selector wrapper over one complete path pattern.
    ///
    /// The child materializes every candidate row. `PathSearch` then applies
    /// endpoint partitioning and hop-count selection for `ANY` / `SHORTEST`
    /// without changing the child join-tree shape.
    PathSearch {
        /// Selector to apply to the child path pattern.
        selector: PathSelector,
        /// Complete path-pattern child.
        child: Box<JoinTree>,
        /// Binding for the first node in the selected path.
        source_binding: TailBinding,
        /// Binding for the final node in the selected path.
        final_binding: TailBinding,
        /// Hop-count contributors in path order.
        hop_contributors: Vec<HopContributor>,
    },
    /// Restrictive path-mode wrapper over one complete path pattern.
    ///
    /// The child materializes every candidate row. `PathModeFilter` then applies
    /// per-row binding-path validation for `TRAIL`, `SIMPLE`, or `ACYCLIC` using
    /// the explicit ordered contributors captured during lowering.
    PathModeFilter {
        /// Restrictive path mode to validate.
        path_mode: PathMode,
        /// Complete path-pattern child.
        child: Box<JoinTree>,
        /// Ordered node and edge contributors in binding-path order.
        path_contributors: Vec<PathContributor>,
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
        /// Predicates scoped to the optional right side.
        right_filters: Vec<FilterPredicate>,
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

/// Executor-private binding slot for anonymous pattern elements.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HiddenBindingId(u32);

impl HiddenBindingId {
    pub(crate) const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Return this hidden slot's zero-based numeric index.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Node or edge scan.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeOrEdgeScan {
    /// Named binding, or `None` for anonymous pattern elements.
    pub binding: Option<BindingId>,
    /// Executor-private slot for anonymous scan elements.
    pub hidden_binding: Option<HiddenBindingId>,
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
    /// Executor-private slot for anonymous edge patterns.
    pub hidden_binding: Option<HiddenBindingId>,
    /// Label predicate attached to the edge.
    pub label_predicate: Option<LabelExpr>,
    /// Inline property predicates from the edge pattern.
    pub property_predicates: Vec<FilterPredicate>,
    /// Binding on the syntactic left side of the edge, if named.
    pub left_binding: Option<BindingId>,
    /// Executor-private slot on the syntactic left side of the edge.
    pub left_hidden_binding: Option<HiddenBindingId>,
    /// Binding on the syntactic right side of the edge, if named.
    pub right_binding: Option<BindingId>,
    /// Executor-private slot on the syntactic right side of the edge.
    pub right_hidden_binding: Option<HiddenBindingId>,
    /// Label predicate on the syntactic right-side node, if any.
    pub right_label_predicate: Option<LabelExpr>,
    /// Property-map equality predicates on the syntactic right-side node.
    pub right_property_predicates: Vec<FilterPredicate>,
    /// Optimizer-selected access path.
    pub access: ScanAccess,
    /// Source span.
    pub span: SourceSpan,
}

/// Quantified edge pattern in a variable-length expansion.
#[derive(Clone, Debug, PartialEq)]
pub struct RepeatEdgeMatch {
    /// Named group edge binding, or `None` for anonymous quantified edges.
    pub group_binding: Option<BindingId>,
    /// Hidden group edge binding for anonymous quantified edges under selectors.
    pub group_hidden_binding: Option<HiddenBindingId>,
    /// Label predicate attached to each traversed edge.
    pub label_predicate: Option<LabelExpr>,
    /// Property-map predicates evaluated against each traversed edge.
    pub property_predicates: Vec<FilterPredicate>,
    /// Inline edge predicates evaluated once per traversed edge.
    pub inline_predicates: Vec<FilterPredicate>,
    /// Binding on the syntactic left side of the repeat, if named.
    pub left_binding: Option<BindingId>,
    /// Executor-private slot on the syntactic left side of the repeat.
    pub left_hidden_binding: Option<HiddenBindingId>,
    /// Binding on the syntactic final node, if named.
    pub final_binding: Option<BindingId>,
    /// Executor-private slot on the syntactic final node.
    pub final_hidden_binding: Option<HiddenBindingId>,
    /// Label predicate on the syntactic final node, if any.
    pub final_label_predicate: Option<LabelExpr>,
    /// Property-map equality predicates on the syntactic final node.
    pub final_property_predicates: Vec<FilterPredicate>,
    /// Optimizer-selected access path for future repeat-aware planning.
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
#[allow(clippy::large_enum_variant)]
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
    /// Match a graph pattern against each incoming row.
    Match(PatternPlan),
    /// Optionally match a graph pattern against each incoming row.
    OptionalMatch(PatternPlan),
    /// Planned procedure call.
    Call(PlannedCall),
    /// Inline `CALL { ... }` table subquery.
    CallSubquery(Box<PlannedTableSubquery>),
    /// Mutation operation.
    Mutation(MutationOp),
    /// Catalog operation.
    Catalog(CatalogOp),
    /// Return a textual dump of an inner execution plan.
    ExplainPlan {
        /// Planned inner statement. It is never executed by this operation.
        inner: Box<ExecutionPlan>,
        /// Source span.
        span: SourceSpan,
    },
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
    /// Maximum unique row keys a set operation may hold while counting rows.
    pub set_op_key_cap: NonZeroUsize,
}

impl ImplDefinedCaps {
    /// Default maximum unique row keys a set operation may hold.
    pub const DEFAULT_SET_OP_KEY_CAP: usize = 1_000_000;

    /// Return the configured set-operation key cap.
    #[must_use]
    pub const fn set_op_key_cap(&self) -> usize {
        self.set_op_key_cap.get()
    }

    /// Return a copy with a different set-operation key cap.
    #[must_use]
    pub const fn with_set_op_key_cap(mut self, set_op_key_cap: NonZeroUsize) -> Self {
        self.set_op_key_cap = set_op_key_cap;
        self
    }
}

impl Default for ImplDefinedCaps {
    fn default() -> Self {
        Self {
            max_quantifier: 100,
            max_optimizer_iterations: 8,
            max_path_length: 32,
            max_wco_traversal_nodes: 64,
            set_op_key_cap: NonZeroUsize::new(Self::DEFAULT_SET_OP_KEY_CAP)
                .expect("default set-op key cap is non-zero"),
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
    /// Executor-private anonymous pattern slot.
    pub hidden: Option<HiddenBindingId>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_join_tree_carries_group_and_final_node_slots_separately() {
        let tree = JoinTree::Repeat {
            child: Box::new(JoinTree::Scan(NodeOrEdgeScan {
                binding: None,
                hidden_binding: Some(HiddenBindingId::new(0)),
                kind: ScanKind::Node,
                label_predicate: None,
                property_predicates: Vec::new(),
                access: ScanAccess::Linear,
                span: SourceSpan::default(),
            })),
            edge: RepeatEdgeMatch {
                group_binding: None,
                group_hidden_binding: None,
                label_predicate: None,
                property_predicates: Vec::new(),
                inline_predicates: Vec::new(),
                left_binding: None,
                left_hidden_binding: Some(HiddenBindingId::new(0)),
                final_binding: None,
                final_hidden_binding: Some(HiddenBindingId::new(1)),
                final_label_predicate: None,
                final_property_predicates: Vec::new(),
                access: ScanAccess::Linear,
                span: SourceSpan::default(),
            },
            direction: EdgeDirection::Right,
            min: 0,
            max: Some(2),
            path_mode: PathMode::Walk,
            selector: None,
        };

        let JoinTree::Repeat { edge, min, max, .. } = tree else {
            panic!("expected repeat");
        };
        assert_eq!(edge.group_binding, None);
        assert_eq!(edge.group_hidden_binding, None);
        assert_eq!(edge.left_hidden_binding, Some(HiddenBindingId::new(0)));
        assert_eq!(edge.final_hidden_binding, Some(HiddenBindingId::new(1)));
        assert_eq!(min, 0);
        assert_eq!(max, Some(2));
    }
}
