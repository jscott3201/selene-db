//! Parser error types and GQLSTATUS mappings.

use std::fmt;

use selene_core::feature_register::FeatureId;

use crate::ast::span::SourceSpan;

/// Five-character ISO GQLSTATUS code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GqlStatus([u8; 5]);

impl GqlStatus {
    /// Syntax error or access rule violation.
    pub const SYNTAX_ERROR: Self = Self(*b"42601");
    /// Feature not supported.
    pub const FEATURE_NOT_SUPPORTED: Self = Self(*b"0A000");
    /// Program limit exceeded.
    pub const PROGRAM_LIMIT_EXCEEDED: Self = Self(*b"54000");

    /// Return this status as its 5-character string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("XX500")
    }
}

impl fmt::Display for GqlStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// GQL parser and flagger error.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum ParserError {
    /// Source text did not parse as supported GQL syntax.
    #[error("{message}")]
    #[diagnostic(code(SLENE_GQL_42601))]
    SyntaxError {
        /// ISO GQLSTATUS code.
        status: GqlStatus,
        /// Human-readable diagnostic message.
        message: String,
        /// Source span for the parse failure.
        #[label("here")]
        span: SourceSpan,
        /// Optional repair hint.
        #[help]
        hint: Option<String>,
    },

    /// Parsed syntax requires a feature outside the current support set.
    #[error("feature not supported: {} ({display_name})", feature_id.as_str())]
    #[diagnostic(code(SLENE_GQL_0A000))]
    UnsupportedFeature {
        /// ISO feature identifier.
        feature_id: FeatureId,
        /// Human-readable feature name.
        display_name: &'static str,
        /// Source span requiring the feature.
        #[label("requires feature")]
        span: SourceSpan,
        /// Static hint for enabling or avoiding the feature.
        #[help]
        hint: &'static str,
    },

    /// Query introduced more distinct identifiers than the parser cap allows.
    #[error("identifier limit exceeded ({limit})")]
    #[diagnostic(code(SLENE_GQL_54000))]
    IdentifierLimitExceeded {
        /// Distinct identifier limit.
        limit: u32,
        /// Source span that crossed the limit.
        #[label("introduces too many new names")]
        span: SourceSpan,
    },

    /// Source parsed at the grammar level, but no AST builder is implemented yet.
    ///
    /// Distinct from [`Self::SyntaxError`] (parse failed) and
    /// [`Self::UnsupportedFeature`] (specific ISO feature not in the claim list).
    /// This variant covers grammar surfaces selene-db will support but whose
    /// builders land in a later brief.
    #[error("not implemented: {message}")]
    #[diagnostic(code(SLENE_GQL_0A000))]
    NotImplemented {
        /// Human-readable description of the missing capability.
        message: String,
        /// Source span requiring the missing capability.
        #[label("not implemented yet")]
        span: SourceSpan,
        /// Pointer to the brief or milestone that lands the capability.
        #[help]
        hint: Option<String>,
    },
}

impl ParserError {
    /// Map this error to its 5-character ISO GQLSTATUS code.
    #[must_use]
    pub const fn gqlstatus(&self) -> GqlStatus {
        match self {
            Self::SyntaxError { status, .. } => *status,
            Self::UnsupportedFeature { .. } | Self::NotImplemented { .. } => {
                GqlStatus::FEATURE_NOT_SUPPORTED
            }
            Self::IdentifierLimitExceeded { .. } => GqlStatus::PROGRAM_LIMIT_EXCEEDED,
        }
    }

    pub(crate) fn syntax(
        message: impl Into<String>,
        span: SourceSpan,
        hint: Option<String>,
    ) -> Self {
        Self::SyntaxError {
            status: GqlStatus::SYNTAX_ERROR,
            message: message.into(),
            span,
            hint,
        }
    }

    pub(crate) fn unsupported_bootstrap(span: SourceSpan) -> Self {
        Self::NotImplemented {
            message: "BRIEF-16 parser bootstrap only supports RETURN literal statements".into(),
            span,
            hint: Some(
                "MATCH, mutation, DDL, CALL, and expression builders land in later briefs".into(),
            ),
        }
    }

    pub(crate) fn empty_program() -> Self {
        Self::syntax(
            "empty GQL program",
            SourceSpan::default(),
            Some("provide a RETURN statement".into()),
        )
    }
}
