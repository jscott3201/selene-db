//! Statement AST nodes.

use selene_core::IStr;

use crate::ast::{expr::ValueExpr, span::SourceSpan};

/// Top-level GQL statement.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum Statement {
    /// `RETURN` query statement.
    Return(ReturnStatement),
}

impl Statement {
    /// Return the source span for this statement.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::Return(statement) => statement.span,
        }
    }
}

/// `RETURN` statement.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ReturnStatement {
    /// Return-list items.
    pub items: Vec<ReturnItem>,
    /// Source span of the full `RETURN` statement.
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
