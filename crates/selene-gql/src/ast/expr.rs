//! Expression AST nodes.

use selene_core::IStr;

use crate::ast::span::SourceSpan;

/// Value expression.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum ValueExpr {
    /// Literal value expression.
    Literal(Literal),
    /// Variable reference. Populated by BRIEF-17 from identifier rules.
    Variable {
        /// Interned variable name.
        name: IStr,
        /// Source span of the variable reference.
        span: SourceSpan,
    },
}

impl ValueExpr {
    /// Return the source span for this expression.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::Literal(literal) => literal.span(),
            Self::Variable { span, .. } => *span,
        }
    }
}

/// Literal expression.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum Literal {
    /// Boolean literal.
    Bool(bool, SourceSpan),
    /// Signed 64-bit integer literal.
    Integer(i64, SourceSpan),
    /// 64-bit floating-point literal.
    Float(f64, SourceSpan),
    /// Interned string literal.
    String(IStr, SourceSpan),
    /// Null literal.
    Null(SourceSpan),
}

impl Literal {
    /// Return the source span for this literal.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        match self {
            Self::Bool(_, span)
            | Self::Integer(_, span)
            | Self::Float(_, span)
            | Self::String(_, span)
            | Self::Null(span) => *span,
        }
    }
}
