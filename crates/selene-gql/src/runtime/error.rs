//! Executor diagnostics and GQLSTATUS mapping.

use crate::{GqlStatus, SourceSpan};

/// Query execution failure.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum ExecutorError {
    /// Runtime data exception such as arithmetic overflow or division by zero.
    #[error("execution data exception: {message}")]
    #[diagnostic(code(SLENE_X_22000))]
    DataException {
        /// Human-readable diagnostic message.
        message: String,
        /// Source span for the failing expression.
        #[label("data exception")]
        span: SourceSpan,
    },

    /// Runtime reference lookup failed for a malformed row or dynamic access.
    #[error("invalid runtime reference: {name}")]
    #[diagnostic(code(SLENE_X_42002))]
    InvalidReference {
        /// Missing or malformed reference name.
        name: String,
        /// Source span requiring the reference.
        #[label("invalid reference")]
        span: SourceSpan,
    },

    /// Implementation-defined executor surface not supported by this brief.
    #[error("implementation-defined executor failure: {detail}")]
    #[diagnostic(code(SLENE_X_XX500))]
    ImplementationDefined {
        /// Stable detail tag asserted by tests.
        detail: &'static str,
    },
}

impl ExecutorError {
    /// Map this executor failure to its GQLSTATUS code.
    #[must_use]
    pub const fn gqlstatus(&self) -> GqlStatus {
        match self {
            Self::DataException { .. } => GqlStatus::DATA_EXCEPTION,
            Self::InvalidReference { .. } => GqlStatus::INVALID_REFERENCE,
            Self::ImplementationDefined { .. } => GqlStatus::IMPLEMENTATION_DEFINED_ERROR,
        }
    }
}
