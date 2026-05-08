//! Analyzer diagnostics.

use selene_core::IStr;

use crate::{GqlStatus, SourceSpan, analyze::binding::BindingDeclKind};

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

    /// A pattern variable is reused with an element kind incompatible with
    /// its prior declaration (e.g. a node variable later used as an edge,
    /// or a path binding aliased over an existing node variable).
    #[error(
        "pattern variable {name} is already bound as a {prior} and cannot be reused as a {current}"
    )]
    #[diagnostic(code(SLENE_GQL_42710))]
    PatternKindMismatch {
        /// Reused binding name.
        name: IStr,
        /// Element kind of the prior declaration.
        prior: PatternElementKind,
        /// Element kind of the new occurrence.
        current: PatternElementKind,
        /// Source span of the new occurrence.
        #[label("incompatible reuse")]
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

/// Pattern element categories used by [`AnalysisError::PatternKindMismatch`].
///
/// The bind pass groups declaration sites by the graph element they introduce
/// (node, edge, path, value). Cross-category reuse via the same name is a
/// semantic error; same-category reuse is allowed (e.g., `MATCH (n)` followed
/// by `INSERT (n)-[:K]->(m)` legitimately reuses `n` as a node variable).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PatternElementKind {
    /// `MATCH (n)` / `INSERT (n)`.
    Node,
    /// `MATCH ()-[e]->()` / `INSERT ()-[e]->()`.
    Edge,
    /// `path = (...)`.
    Path,
}

impl PatternElementKind {
    /// Categorize a [`BindingDeclKind`] for compatibility checks.
    #[must_use]
    pub const fn from_decl_kind(kind: BindingDeclKind) -> Option<Self> {
        match kind {
            BindingDeclKind::NodePattern | BindingDeclKind::InsertNode => Some(Self::Node),
            BindingDeclKind::EdgePattern | BindingDeclKind::InsertEdge => Some(Self::Edge),
            BindingDeclKind::PathBinding => Some(Self::Path),
            BindingDeclKind::LetAlias
            | BindingDeclKind::UnwindAlias
            | BindingDeclKind::ProjectionAlias
            | BindingDeclKind::YieldColumn => None,
        }
    }
}

impl std::fmt::Display for PatternElementKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Node => "node variable",
            Self::Edge => "edge variable",
            Self::Path => "path variable",
        })
    }
}

impl AnalysisError {
    /// Return this error's ISO GQLSTATUS code.
    #[must_use]
    pub const fn gqlstatus(&self) -> GqlStatus {
        match self {
            Self::UndefinedReference { .. } => GqlStatus::UNDEFINED_REFERENCE,
            Self::Shadow { .. } | Self::PatternKindMismatch { .. } => GqlStatus::DUPLICATE_OBJECT,
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
