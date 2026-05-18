//! Executor diagnostics and GQLSTATUS mapping.

use crate::{GqlStatus, ProcedureError, SourceSpan};

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

    /// Transaction state does not permit the requested executor operation.
    #[error("invalid transaction state: {detail}")]
    #[diagnostic(code(SLENE_X_25000))]
    InvalidTransactionState {
        /// Stable detail tag asserted by tests.
        detail: &'static str,
        /// Source span requiring a write transaction.
        #[label("invalid transaction state")]
        span: SourceSpan,
    },

    /// `START TRANSACTION` was requested while an explicit transaction exists.
    #[error("transaction already active")]
    #[diagnostic(code(SLENE_X_25000))]
    TransactionAlreadyActive {
        /// Source span for the transaction-control statement.
        #[label("transaction already active")]
        span: SourceSpan,
    },

    /// `COMMIT` or `ROLLBACK` was requested without an explicit transaction.
    #[error("no active transaction")]
    #[diagnostic(code(SLENE_X_25000))]
    NoActiveTransaction {
        /// Source span for the transaction-control statement.
        #[label("no active transaction")]
        span: SourceSpan,
    },

    /// Statement was issued while the explicit transaction is aborted.
    #[error("statement issued against aborted explicit transaction")]
    #[diagnostic(code(SLENE_X_25P02))]
    InFailedTransaction {
        /// Source span for the rejected statement.
        #[label("aborted transaction; issue ROLLBACK to recover")]
        span: SourceSpan,
    },

    /// The graph mutation funnel rejected a write.
    #[error("graph mutation failed: {source}")]
    #[diagnostic(code(SLENE_X_XX501))]
    GraphMutation {
        /// Underlying graph-layer error.
        #[source]
        source: selene_graph::GraphError,
        /// Source span for the write site.
        #[label("graph mutation failed")]
        span: SourceSpan,
    },

    /// Commit-critical durability provider flush failed.
    #[error("durability flush failed for provider {provider_tag}: {reason}")]
    #[diagnostic(code(SLENE_X_XX502))]
    Flush {
        /// Durable provider tag.
        provider_tag: selene_graph::ProviderTag,
        /// Human-readable provider failure reason.
        reason: String,
    },

    /// Procedure registry execution failed.
    #[error("procedure execution failed: {source}")]
    #[diagnostic(code(SLENE_X_PROC))]
    Procedure {
        /// Underlying procedure error.
        #[source]
        source: ProcedureError,
        /// Source span for the CALL site.
        #[label("procedure failed")]
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
            Self::InvalidTransactionState { .. }
            | Self::TransactionAlreadyActive { .. }
            | Self::NoActiveTransaction { .. }
            | Self::InFailedTransaction { .. } => GqlStatus::INVALID_TRANSACTION_STATE,
            Self::GraphMutation { .. } | Self::Flush { .. } => {
                GqlStatus::IMPLEMENTATION_DEFINED_ERROR
            }
            Self::Procedure { source, .. } => source.gqlstatus(),
            Self::ImplementationDefined { .. } => GqlStatus::IMPLEMENTATION_DEFINED_ERROR,
        }
    }
}
