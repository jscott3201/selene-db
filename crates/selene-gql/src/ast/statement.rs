//! Statement AST nodes.

use selene_core::IStr;

use crate::ast::{
    call::{InlineProcedureCall, ProcedureCall},
    ddl::DdlStatement,
    expr::ValueExpr,
    mutation::MutationPipeline,
    pattern::MatchClause,
    span::SourceSpan,
    types::GqlType,
    util::NonEmpty,
};

/// Top-level GQL statement.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum Statement {
    /// Read query pipeline.
    Query(QueryPipeline),
    /// Set-composed read pipelines.
    Composite {
        /// First pipeline.
        first: QueryPipeline,
        /// Remaining pipelines paired with their set operator; type-enforced
        /// non-empty because a composite statement has at least one set op.
        rest: NonEmpty<(SetOp, QueryPipeline)>,
        /// Source span.
        span: SourceSpan,
    },
    /// `NEXT`-chained read pipelines.
    Chained {
        /// Chained pipeline blocks.
        blocks: Vec<QueryPipeline>,
        /// Source span.
        span: SourceSpan,
    },
    /// Write-side mutation pipeline.
    Mutate(MutationPipeline),
    /// Data-definition statement.
    Ddl(DdlStatement),
    /// Top-level procedure call.
    Call(ProcedureCall),
    /// `EXPLAIN <statement>`.
    ///
    /// Plans the inner statement and returns a textual plan without executing
    /// the inner statement.
    Explain {
        /// Inner statement to plan.
        inner: Box<Statement>,
        /// Source span.
        span: SourceSpan,
    },
    /// `START TRANSACTION`.
    StartTransaction {
        /// Source span.
        span: SourceSpan,
    },
    /// `COMMIT`.
    Commit {
        /// Source span.
        span: SourceSpan,
    },
    /// `ROLLBACK`.
    Rollback {
        /// Source span.
        span: SourceSpan,
    },
}

impl Statement {
    /// Return the source span for this statement.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::Query(pipeline) => pipeline.span,
            Self::Composite { span, .. } | Self::Chained { span, .. } => *span,
            Self::Mutate(pipeline) => pipeline.span,
            Self::Ddl(statement) => statement.span(),
            Self::Call(call) => call.span,
            Self::Explain { span, .. } => *span,
            Self::StartTransaction { span } | Self::Commit { span } | Self::Rollback { span } => {
                *span
            }
        }
    }
}

/// Set operator.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum SetOp {
    /// `UNION` (distinct).
    Union,
    /// `UNION ALL` (multiset).
    UnionAll,
    /// `INTERSECT` (distinct).
    Intersect,
    /// `INTERSECT ALL` (multiset).
    IntersectAll,
    /// `EXCEPT` (distinct).
    Except,
    /// `EXCEPT ALL` (multiset).
    ExceptAll,
    /// `OTHERWISE`.
    Otherwise,
}

/// Read query pipeline.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct QueryPipeline {
    /// Ordered pipeline statements.
    pub statements: Vec<PipelineStatement>,
    /// Source span.
    pub span: SourceSpan,
}

/// One read-side pipeline statement.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum PipelineStatement {
    /// `MATCH`.
    Match(MatchClause),
    /// `FILTER`.
    Filter(ValueExpr),
    /// `LET`.
    Let(Vec<LetBinding>),
    /// `UNWIND`.
    Unwind(UnwindStatement),
    /// `ORDER BY`.
    Sorting(Vec<OrderTerm>),
    /// `LIMIT`.
    Limit(LimitValue),
    /// `OFFSET` / `SKIP`.
    Offset(LimitValue),
    /// `RETURN`.
    Return(ReturnClause),
    /// `WITH`.
    With(WithClause),
    /// `CALL`.
    Call(ProcedureCall),
    /// Inline `CALL { ... }`.
    CallSubquery(InlineProcedureCall),
}

impl PipelineStatement {
    /// Return this statement's source span.
    #[must_use]
    pub fn span(&self) -> SourceSpan {
        match self {
            Self::Match(value) => value.span,
            Self::Filter(value) => value.span(),
            Self::Let(values) => span_from_iter(values.iter().map(|value| value.span)),
            Self::Unwind(value) => value.span,
            Self::Sorting(values) => span_from_iter(values.iter().map(|value| value.span)),
            Self::Limit(value) | Self::Offset(value) => value.span(),
            Self::Return(value) => value.span,
            Self::With(value) => value.span,
            Self::Call(value) => value.span,
            Self::CallSubquery(value) => value.span,
        }
    }
}

/// Variable binding in `LET`.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct LetBinding {
    /// Interned alias.
    pub alias: IStr,
    /// Bound value expression.
    pub value: ValueExpr,
    /// Source span.
    pub span: SourceSpan,
}

/// `UNWIND` statement.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct UnwindStatement {
    /// Source expression.
    pub source: ValueExpr,
    /// Interned alias.
    pub alias: IStr,
    /// Source span.
    pub span: SourceSpan,
}

/// `ORDER BY` term.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct OrderTerm {
    /// Sorted expression.
    pub expr: ValueExpr,
    /// Sort direction.
    pub direction: OrderDirection,
    /// Optional null-order policy.
    pub nulls: Option<NullsPolicy>,
    /// Source span.
    pub span: SourceSpan,
}

/// Sort direction.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize,
)]
pub enum OrderDirection {
    /// Ascending.
    #[default]
    Asc,
    /// Descending.
    Desc,
}

/// Null ordering policy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum NullsPolicy {
    /// `NULLS FIRST`.
    NullsFirst,
    /// `NULLS LAST`.
    NullsLast,
}

/// `LIMIT` / `OFFSET` value.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum LimitValue {
    /// Literal count.
    Count(u64, SourceSpan),
    /// Parameter reference.
    Parameter {
        /// Interned parameter name without the leading `$`.
        name: IStr,
        /// Optional inline declared parameter type.
        declared_type: Option<GqlType>,
        /// Source span of the parameter reference.
        span: SourceSpan,
    },
}

impl LimitValue {
    /// Return source span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::Count(_, span) | Self::Parameter { span, .. } => *span,
        }
    }
}

/// `RETURN` clause.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ReturnClause {
    /// `DISTINCT`.
    pub distinct: bool,
    /// `RETURN *`.
    pub star: bool,
    /// Return-list items. Empty when `star` is true.
    pub items: Vec<ReturnItem>,
    /// Optional `GROUP BY` list.
    pub group_by: Option<Vec<ValueExpr>>,
    /// Optional `HAVING` condition.
    pub having: Option<ValueExpr>,
    /// Source span.
    pub span: SourceSpan,
}

/// One `RETURN` projection item.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ReturnItem {
    /// Returned expression.
    pub expr: ValueExpr,
    /// Optional `AS` alias.
    pub alias: Option<IStr>,
    /// Source span of the projection item.
    pub span: SourceSpan,
}

/// `WITH` clause.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct WithClause {
    /// `DISTINCT`.
    pub distinct: bool,
    /// Projected items.
    pub items: Vec<ReturnItem>,
    /// Optional `GROUP BY` list.
    pub group_by: Option<Vec<ValueExpr>>,
    /// Optional `HAVING` condition.
    pub having: Option<ValueExpr>,
    /// Optional post-projection `WHERE`.
    pub where_clause: Option<ValueExpr>,
    /// Source span.
    pub span: SourceSpan,
}

/// Helper for type-bearing DDL statements.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct TypedBinding {
    /// Binding name.
    pub name: IStr,
    /// Parsed type.
    pub ty: GqlType,
    /// Source span.
    pub span: SourceSpan,
}

fn span_from_iter(mut spans: impl Iterator<Item = SourceSpan>) -> SourceSpan {
    let Some(first) = spans.next() else {
        return SourceSpan::default();
    };
    spans.fold(first, SourceSpan::merge)
}
