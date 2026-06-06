//! Procedure-call AST nodes.

use selene_core::DbString;

use crate::ast::{expr::ValueExpr, span::SourceSpan, statement::QueryPipeline, util::NonEmpty};

/// Top-level or in-pipeline procedure call.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProcedureCall {
    /// Qualified procedure name as database-string path segments.
    pub name: NonEmpty<DbString>,
    /// Positional arguments.
    pub args: Vec<ValueExpr>,
    /// Requested yield columns. Empty means the call discards return columns.
    pub yield_items: Vec<YieldItem>,
    /// Source span.
    pub span: SourceSpan,
}

/// Inline `CALL { ... }` query subquery.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct InlineProcedureCall {
    /// Optional explicit variable-scope names from `CALL (x, y) { ... }`.
    pub variable_scope: Option<Vec<DbString>>,
    /// Query body executed per input row.
    pub body: Box<QueryPipeline>,
    /// Requested yield columns. Empty means the call discards return columns.
    pub yield_items: Vec<YieldItem>,
    /// Whether the source requested `IN TRANSACTIONS`.
    pub in_transactions: bool,
    /// Source span.
    pub span: SourceSpan,
}

/// One `YIELD` item.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct YieldItem {
    /// Yielded column.
    pub column: YieldColumn,
    /// Optional alias.
    pub alias: Option<DbString>,
    /// Source span.
    pub span: SourceSpan,
}

/// Yielded column selector.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum YieldColumn {
    /// `YIELD *`.
    Star,
    /// `YIELD col`.
    Named(DbString),
}
