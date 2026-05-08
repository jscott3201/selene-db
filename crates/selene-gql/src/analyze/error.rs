//! Analyzer diagnostics.

use selene_core::IStr;

use crate::{GqlStatus, SourceSpan};

/// Semantic-analysis failure.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum AnalysisError {
    /// A reference does not resolve to any binding in the enclosing scopes.
    #[error("undefined reference: {name}")]
    #[diagnostic(code(SLENE_GQL_42703))]
    UndefinedReference {
        /// Unresolved binding name.
        name: IStr,
        /// Source span of the unresolved reference.
        #[label("not bound in scope")]
        span: SourceSpan,
        /// Optional repair hint.
        #[help]
        hint: Option<String>,
    },

    /// A strict declaration site redeclared a binding already present in its scope.
    #[error("binding {name} is already declared in this scope")]
    #[diagnostic(code(SLENE_GQL_42710))]
    Shadow {
        /// Redeclared binding name.
        name: IStr,
        /// Source span of the redeclaration.
        #[label("conflicts with an earlier binding")]
        span: SourceSpan,
        /// Source span of the prior declaration.
        #[label("first declared here")]
        prior_span: SourceSpan,
    },

    /// The analyzer encountered an AST surface it does not route yet.
    #[error("not implemented: {message}")]
    #[diagnostic(code(SLENE_GQL_0A000))]
    NotImplemented {
        /// Human-readable missing capability.
        message: String,
        /// Source span requiring the missing analyzer capability.
        #[label("not implemented yet")]
        span: SourceSpan,
        /// Optional implementation hint.
        #[help]
        hint: Option<String>,
    },
}

impl AnalysisError {
    /// Return this error's ISO GQLSTATUS code.
    #[must_use]
    pub const fn gqlstatus(&self) -> GqlStatus {
        match self {
            Self::UndefinedReference { .. } => GqlStatus::UNDEFINED_REFERENCE,
            Self::Shadow { .. } => GqlStatus::DUPLICATE_OBJECT,
            Self::NotImplemented { .. } => GqlStatus::FEATURE_NOT_SUPPORTED,
        }
    }

    pub(crate) fn undefined_reference(name: IStr, span: SourceSpan) -> Self {
        Self::UndefinedReference {
            name,
            span,
            hint: Some("declare the variable before this reference".into()),
        }
    }
}
