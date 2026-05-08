//! Mutation statement AST nodes.

use selene_core::IStr;

use crate::ast::{
    expr::ValueExpr,
    pattern::{GraphPattern, MatchClause},
    span::SourceSpan,
    statement::ReturnClause,
};

/// Write-side mutation pipeline.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct MutationPipeline {
    /// Ordered read and mutation statements.
    pub statements: Vec<MutationStatement>,
    /// Optional `RETURN` or `FINISH` terminator.
    pub terminator: Option<MutationTerminator>,
    /// Source span.
    pub span: SourceSpan,
}

/// One statement inside a mutation pipeline.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum MutationStatement {
    /// `MATCH`.
    Match(MatchClause),
    /// `FILTER`.
    Filter(ValueExpr),
    /// `INSERT`.
    Insert(InsertStatement),
    /// `SET`.
    Set(Vec<SetItem>),
    /// `REMOVE`.
    Remove(Vec<RemoveItem>),
    /// `DELETE` / `DETACH DELETE` / `NODETACH DELETE`.
    Delete(DeleteStatement),
}

impl MutationStatement {
    /// Return this statement's source span.
    #[must_use]
    pub fn span(&self) -> SourceSpan {
        match self {
            Self::Match(value) => value.span,
            Self::Filter(value) => value.span(),
            Self::Insert(value) => value.span,
            Self::Set(values) => span_from_iter(values.iter().map(SetItem::span)),
            Self::Remove(values) => span_from_iter(values.iter().map(RemoveItem::span)),
            Self::Delete(value) => value.span,
        }
    }
}

/// Mutation pipeline terminator.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum MutationTerminator {
    /// `RETURN`.
    Return(ReturnClause),
    /// `FINISH`.
    Finish(SourceSpan),
}

impl MutationTerminator {
    /// Return this terminator's source span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::Return(value) => value.span,
            Self::Finish(span) => *span,
        }
    }
}

/// `INSERT` graph pattern.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct InsertStatement {
    /// Inserted graph patterns.
    pub patterns: Vec<GraphPattern>,
    /// Source span.
    pub span: SourceSpan,
}

/// One `SET` item.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum SetItem {
    /// `SET n.prop = expr`.
    Property {
        /// Bound target variable.
        target: IStr,
        /// Property key.
        key: IStr,
        /// New value.
        value: ValueExpr,
        /// Source span.
        span: SourceSpan,
    },
    /// `SET n = {k: v}`.
    PropertyMerge {
        /// Bound target variable.
        target: IStr,
        /// Replacement/merge properties.
        properties: Vec<(IStr, ValueExpr)>,
        /// Source span.
        span: SourceSpan,
    },
    /// `SET n :Label` / `SET n IS Label`.
    Label {
        /// Bound target variable.
        target: IStr,
        /// Label name.
        label: IStr,
        /// Source span.
        span: SourceSpan,
    },
}

impl SetItem {
    /// Return this item's source span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::Property { span, .. }
            | Self::PropertyMerge { span, .. }
            | Self::Label { span, .. } => *span,
        }
    }
}

/// One `REMOVE` item.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum RemoveItem {
    /// `REMOVE n.prop`.
    Property {
        /// Bound target variable.
        target: IStr,
        /// Property key.
        key: IStr,
        /// Source span.
        span: SourceSpan,
    },
    /// `REMOVE n :Label` / `REMOVE n IS Label`.
    Label {
        /// Bound target variable.
        target: IStr,
        /// Label name.
        label: IStr,
        /// Source span.
        span: SourceSpan,
    },
}

impl RemoveItem {
    /// Return this item's source span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::Property { span, .. } | Self::Label { span, .. } => *span,
        }
    }
}

/// `DELETE` statement.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct DeleteStatement {
    /// Delete mode.
    pub mode: DeleteMode,
    /// Bound variables to delete.
    pub items: Vec<IStr>,
    /// Source span.
    pub span: SourceSpan,
}

/// Delete behavior requested by syntax.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum DeleteMode {
    /// Bare `DELETE`.
    Bare,
    /// `DETACH DELETE`.
    Detach,
    /// `NODETACH DELETE`.
    NoDetach,
}

fn span_from_iter(mut spans: impl Iterator<Item = SourceSpan>) -> SourceSpan {
    let Some(first) = spans.next() else {
        return SourceSpan::default();
    };
    spans.fold(first, SourceSpan::merge)
}
