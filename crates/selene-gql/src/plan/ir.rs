//! Planner IR definitions.

use selene_core::IStr;

use crate::{
    EdgeDirection, LabelExpr, NullsPolicy, OrderDirection, ProcedureHandle, ProcedureMutability,
    ProcedureOutputSchema, SetOp, SourceSpan, ValueExpr,
    analyze::{AnalyzedType, BindingId, ExprId},
};

/// Literal execution plan produced by BRIEF-26 lowering.
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
    pub name: IStr,
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
        key: Vec<IStr>,
    },
    /// Left-outer join used for OPTIONAL MATCH.
    Outer {
        /// Preserved left input.
        left: Box<JoinTree>,
        /// Optional right input.
        right: Box<JoinTree>,
        /// Shared binding names used as the join key.
        key: Vec<IStr>,
    },
    /// Marker for future WCO rewrites.
    WorstCaseOptimal {
        /// Intersected subplans.
        intersection: Vec<JoinTree>,
    },
    /// Nested subplan placeholder.
    Subplan(Box<ExecutionPlan>),
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
    /// Source span.
    pub span: SourceSpan,
}

/// Pipeline operation over binding tables.
#[derive(Clone, Debug)]
pub enum PipelineOp {
    /// Retain rows satisfying a predicate.
    Filter(FilterPredicate),
    /// Project expressions into output columns.
    Project(Vec<ProjectExpr>),
    /// Expand a list expression to one row per element.
    Unwind {
        /// Source list expression.
        source: ProjectExpr,
        /// Alias bound to each list element.
        alias: IStr,
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
    /// Planned procedure call, fully populated in BRIEF-27.
    Call(PlannedCall),
    /// Mutation operation, fully lowered in BRIEF-27.
    Mutation(MutationOp),
}

/// Limit or offset value carried to execution time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LimitAmount {
    /// Literal row count.
    Literal(u64),
    /// Parameter resolved by the executor.
    Parameter(IStr),
}

/// Planned predicate.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterPredicate {
    /// Predicate expression or property-map value expression.
    pub expr: ValueExpr,
    /// Analyzer expression ID for `expr`.
    pub expr_id: ExprId,
    /// Analyzer-inferred type for `expr`.
    pub ty: AnalyzedType,
    /// Referenced bindings, sorted and deduplicated.
    pub binding_refs: Vec<BindingId>,
    /// Predicate shape.
    pub kind: FilterPredicateKind,
    /// Source span.
    pub span: SourceSpan,
}

/// Predicate shape.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FilterPredicateKind {
    /// Ordinary boolean expression.
    Expression,
    /// Property-map equality predicate attached to a node or edge pattern.
    PropertyEquals {
        /// Pattern element binding, if named.
        binding: Option<BindingId>,
        /// Property key.
        key: IStr,
    },
}

/// Planned projection expression.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectExpr {
    /// Projected expression.
    pub expr: ValueExpr,
    /// Analyzer expression ID for `expr`.
    pub expr_id: ExprId,
    /// Analyzer-inferred type for `expr`.
    pub ty: AnalyzedType,
    /// Output alias, when present.
    pub alias: Option<IStr>,
    /// Referenced bindings, sorted and deduplicated.
    pub binding_refs: Vec<BindingId>,
    /// Source span.
    pub span: SourceSpan,
}

/// Planned sort key.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderKey {
    /// Sorted expression.
    pub expr: ValueExpr,
    /// Analyzer expression ID for `expr`.
    pub expr_id: ExprId,
    /// Analyzer-inferred type for `expr`.
    pub ty: AnalyzedType,
    /// Sort direction.
    pub direction: OrderDirection,
    /// Optional null ordering policy.
    pub nulls: Option<NullsPolicy>,
    /// Referenced bindings, sorted and deduplicated.
    pub binding_refs: Vec<BindingId>,
    /// Source span.
    pub span: SourceSpan,
}

/// Planned aggregate call.
#[derive(Clone, Debug, PartialEq)]
pub struct Aggregate {
    /// Aggregate function name.
    pub function: IStr,
    /// Aggregate arguments.
    pub args: Vec<AggregateArg>,
    /// Whether the aggregate uses `*`.
    pub star: bool,
    /// Whether arguments are distinct.
    pub distinct: bool,
    /// Source span.
    pub span: SourceSpan,
}

/// Planned aggregate argument.
#[derive(Clone, Debug, PartialEq)]
pub struct AggregateArg {
    /// Argument expression.
    pub expr: ValueExpr,
    /// Analyzer expression ID for `expr`.
    pub expr_id: ExprId,
    /// Analyzer-inferred type for `expr`.
    pub ty: AnalyzedType,
}

/// Planned procedure call placeholder.
#[derive(Clone, Debug)]
pub struct PlannedCall {
    /// Opaque procedure handle.
    pub handle: ProcedureHandle,
    /// Planned call arguments.
    pub args: Vec<ProjectExpr>,
    /// Requested yield columns.
    pub yield_cols: Vec<PlannedYieldItem>,
    /// Procedure output schema.
    pub output_schema: ProcedureOutputSchema,
    /// Procedure mutability class.
    pub mutability: ProcedureMutability,
    /// Source span.
    pub span: SourceSpan,
}

/// Planned yield item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedYieldItem {
    /// Source output column.
    pub column: IStr,
    /// Output alias, when present.
    pub alias: Option<IStr>,
}

/// Mutation plan variants reserved for BRIEF-27.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MutationOp {
    /// Insert node placeholder.
    InsertNode,
    /// Insert edge placeholder.
    InsertEdge,
    /// Set property placeholder.
    SetProperty,
    /// Set label placeholder.
    SetLabel,
    /// Remove property placeholder.
    RemoveProperty,
    /// Remove label placeholder.
    RemoveLabel,
    /// Delete target placeholder.
    DeleteTarget,
}

/// Planner implementation-defined limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImplDefinedCaps {
    /// Maximum accepted variable-length quantifier upper bound.
    pub max_quantifier: u32,
    /// Fixed-point optimizer iteration cap.
    pub max_optimizer_iterations: u32,
    /// Default maximum path length for future path execution.
    pub max_path_length: u32,
}

impl Default for ImplDefinedCaps {
    fn default() -> Self {
        Self {
            max_quantifier: 100,
            max_optimizer_iterations: 8,
            max_path_length: 32,
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
    pub name: Option<IStr>,
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
