//! Expression-bearing planner IR rows.

use selene_core::DbString;

use crate::{
    GqlType, NullsPolicy, OrderDirection, SourceSpan, ValueExpr,
    analyze::{AnalyzedType, BindingId, ExprId},
};

use super::OrderAccess;

/// Limit or offset value carried to execution time.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LimitAmount {
    /// Literal row count.
    Literal(u64),
    /// Parameter resolved by the executor.
    Parameter {
        /// Database-string parameter name without the leading `$`.
        name: DbString,
        /// Optional inline declared parameter type.
        declared_type: Option<GqlType>,
        /// Source span of the parameter reference.
        span: SourceSpan,
    },
}

/// Planned predicate.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterPredicate {
    /// Predicate expression or property-map value expression.
    pub expr: ValueExpr,
    /// Analyzer expression ID for `expr`.
    ///
    /// Stable identifier carried through the plan for diagnostics, metrics,
    /// and tracing. Optimizer rules that synthesize predicates allocate fresh
    /// IDs from the owning `ExecutionPlan`, preserving the `expr_id` to
    /// expression bijection after lowering. The plan does not retain the
    /// analyzer's `ExprTypeTable`, so post-plan consumers must read `ty`
    /// directly instead of indexing back into analyzer-owned storage.
    pub expr_id: ExprId,
    /// Analyzer-inferred type for `expr`.
    pub ty: AnalyzedType,
    /// Referenced bindings, sorted and deduplicated.
    pub binding_refs: Vec<BindingId>,
    /// Predicate shape.
    pub kind: FilterPredicateKind,
    /// Whether an index-aware rule proved this predicate fully covered.
    pub index_consumed: bool,
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
        key: DbString,
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
    /// Explicit declared target type when this expression must be checked at runtime.
    pub declared_type: Option<GqlType>,
    /// Output alias, when present.
    pub alias: Option<DbString>,
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
    /// Optimizer-selected order access hint.
    pub access: Option<OrderAccess>,
    /// Source span.
    pub span: SourceSpan,
}

/// Planned aggregate call.
#[derive(Clone, Debug, PartialEq)]
pub struct Aggregate {
    /// Analyzer expression ID for the aggregate call.
    pub aggregate_id: ExprId,
    /// Executor-private synthesized output column name.
    pub output_name: DbString,
    /// Aggregate function name.
    pub function: DbString,
    /// Aggregate arguments.
    pub args: Vec<AggregateArg>,
    /// Whether the aggregate uses `*`.
    pub star: bool,
    /// Whether arguments are distinct.
    pub distinct: bool,
    /// Analyzer-inferred aggregate result type.
    pub ty: AnalyzedType,
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
