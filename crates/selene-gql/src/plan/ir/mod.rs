//! Planner IR definitions.

mod access;
mod call;
mod caps;
mod catalog;
mod execution;
mod filter;
mod mutation;
mod session;
mod subquery;
mod tx;

use crate::{
    EdgeDirection, LabelExpr, MatchMode, PathMode, PathSelector, SetOp, SourceSpan,
    analyze::{AnalyzedType, BindingId},
};

pub use access::{IndexKey, NodeIdOrdering, OrderAccess, ScanAccess, TypedIndexBounds};
pub use call::{PlannedCall, PlannedYieldItem, YieldKind};
pub use caps::ImplDefinedCaps;
pub use catalog::{CatalogOp, PlannedTypePropertyConstraint, PlannedTypePropertyDef};
pub use execution::{ExecutionPlan, PipelineOpId};
pub use filter::{
    Aggregate, AggregateArg, FilterPredicate, FilterPredicateKind, LimitAmount, OrderKey,
    ProjectExpr,
};
pub use mutation::{DeleteTargetPlan, InsertEndpointRef, InsertSiteId, MutationOp, PropertyInit};
pub use session::SessionOp;
pub use subquery::{
    OuterBindingRef, PlannedSubquery, PlannedTableSubquery, PlannedTableSubqueryYield,
    SubqueryBody, SubqueryKind, SubqueryRegistry,
};
pub use tx::TxOp;

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
    /// Database-string binding name.
    pub name: selene_core::DbString,
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
    /// One-row, all-null anchor used to model leading optional graph patterns.
    Unit,
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
    /// Pattern-wide match-mode wrapper over a whole `<graph pattern>`.
    ///
    /// Per ISO/IEC 39075:2024 §16.4, a `<match mode>` is a prefix on the entire
    /// comma-separated path-pattern list of one MATCH (not a per-path-pattern
    /// modifier like [`Self::PathModeFilter`]). The child materializes every
    /// candidate row across all path patterns; this wrapper then applies the
    /// pattern-wide edge-uniqueness filter for `DIFFERENT EDGES` (§16.4 GR4 /
    /// GR8(a)) over the union of every edge column in the row. `REPEATABLE
    /// ELEMENTS` installs no filter (GR8(b): BINDINGS = INNER), so the lowering
    /// never constructs this variant for that mode.
    MatchModeFilter {
        /// Match mode to enforce. Only [`MatchMode::DifferentEdges`] reaches a
        /// constructed wrapper; the variant is kept mode-tagged for EXPLAIN and
        /// for an exhaustive runtime match.
        match_mode: MatchMode,
        /// Complete graph-pattern child spanning all path patterns of the MATCH.
        child: Box<JoinTree>,
        /// Every edge contributor across all path patterns, in binding-path
        /// order. The union enables pattern-wide deduplication per §16.4 GR4.
        path_contributors: Vec<PathContributor>,
    },
    /// Binary join between two pattern fragments.
    HashJoin {
        /// Left input.
        left: Box<JoinTree>,
        /// Right input.
        right: Box<JoinTree>,
        /// Shared binding names used as the join key.
        key: Vec<selene_core::DbString>,
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
        key: Vec<selene_core::DbString>,
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
    /// A fully nested [`ExecutionPlan`] executed as a single join-tree node.
    ///
    /// Reserved for correlated-`CALL` subquery lowering: a `CALL { ... }` whose
    /// body imports outer bindings will lower to this variant so the inner
    /// pipeline runs per outer row. No production lowering rule constructs it
    /// yet (correlated-`CALL` is not lowered at HEAD; only tests build it), but
    /// the runtime path is complete — the sole executor is the runtime subplan
    /// executor reached from the `JoinTree::Subplan` arm of pattern walking. It
    /// is kept (not removed) as the working substrate for that near-term
    /// direction.
    Subplan(Box<ExecutionPlan>),
    /// Per-label sub-scans wrapping a flat-disjunctive-label pattern.
    ///
    /// Emitted by the `disjunctive_label_expansion` optimizer rule when a node
    /// scan carries a flat `LabelExpr::Disjunction([Single, Single, …])` label
    /// expression and at least one per-label branch has an applicable typed,
    /// composite, or in-list index. Each branch is a clone of the original
    /// scan with `label_predicate = Some(LabelExpr::Single(L_i))`, allowing the
    /// downstream index-selection rules (`composite_index_lookup`,
    /// `in_list_optimization`, `range_index_scan`) to set per-branch
    /// `ScanAccess` independently.
    ///
    /// Runtime executes each branch via the standard `scan_pattern` entry and
    /// concatenates the per-branch `Binding` rows with `UNION ALL` semantics
    /// (no dedup; a node carrying labels A AND B appears in both branches'
    /// candidate sets, matching the manual `MATCH (n:A) UNION ALL MATCH (n:B)`
    /// behaviour). Per-branch label filtering applies via the existing
    /// `label_matches_scan` machinery against each branch's single-label
    /// predicate.
    DisjunctiveScan {
        /// Per-label sub-scans, each with `label_predicate =
        /// Some(LabelExpr::Single(L_i))` and a clone of the original scan's
        /// property predicates + bindings.
        ///
        /// `Vec`, not `Vec2OrMore`, because the source
        /// `LabelExpr::Disjunction(Vec2OrMore<LabelExpr>)` already guarantees
        /// `≥ 2` branches at construction time.
        branches: Vec<NodeOrEdgeScan>,
        /// The original scan, retained for EXPLAIN diagnostics and to preserve
        /// the original disjunctive `label_predicate` for post-commit walks.
        /// Carries the same `binding` / `hidden_binding` IDs that the branches
        /// inherit, so downstream pipeline ops resolve `(n)` against the
        /// unioned binding table consistently.
        scan_anchor: NodeOrEdgeScan,
    },
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
/// subquery form, INDEX DDL) can add variants without breaking downstream
/// pattern matches.
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
        alias: selene_core::DbString,
        /// Optional position output for ISO `FOR`.
        position: Option<crate::RowExpansionPosition>,
        /// Source span.
        span: SourceSpan,
    },
    /// Sort rows.
    OrderBy(Vec<OrderKey>),
    /// Drop the carrier columns appended so `ORDER BY` could reach a binding
    /// the `RETURN` discards.
    ///
    /// ISO/IEC 39075:2024 §14.10 SR 4)c)i)2)A)VIII appends `REF AS REF` to a
    /// copy of the return item list for every sort-key reference that is not
    /// already a return alias, and GR 1)b)ii sets the working table to a copy
    /// without exactly those columns once the ordering and page statement has
    /// run. Carriers are appended after the projected columns, so dropping them
    /// is a truncation to `projected_width`.
    ///
    /// Positional rather than by name because a return item need not have an
    /// alias: `RETURN d.tag` produces a column whose name is `None`, which no
    /// name-keyed trim could reproduce.
    TrimOrderCarriers {
        /// Number of leading columns the `RETURN` actually projects.
        projected_width: usize,
    },
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
    /// Evaluate a NEXT block once per input row because it references prior bindings.
    CorrelatedChain(Box<ExecutionPlan>),
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
    /// Session-control operation (ISO/IEC 39075:2024 section 7).
    Session(SessionOp),
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
    pub name: Option<selene_core::DbString>,
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
