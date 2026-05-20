//! Executor diagnostics and GQLSTATUS mapping.

use std::time::{Duration, Instant};

use selene_core::IStr;

use crate::{AnalysisError, GqlStatus, ParserError, PlannerError, ProcedureError, SourceSpan};

/// Query execution failure.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum ExecutorError {
    /// Source text failed to parse before execution.
    #[error("parse failed: {source}")]
    #[diagnostic(code(SLENE_X_PARSE))]
    Parse {
        /// Underlying parser error.
        #[source]
        source: ParserError,
    },

    /// Semantic analysis failed before execution.
    #[error("analysis failed: {source}")]
    #[diagnostic(code(SLENE_X_ANALYZE))]
    Analysis {
        /// Underlying analyzer error.
        #[source]
        source: AnalysisError,
    },

    /// Planning failed before execution.
    #[error("planning failed: {source}")]
    #[diagnostic(code(SLENE_X_PLAN))]
    Plan {
        /// Underlying planner error.
        #[source]
        source: PlannerError,
    },

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

    /// A `$name` parameter was referenced but not bound on the session.
    ///
    /// Maps to GQLSTATUS 22G03 per ISO/IEC 39075:2024 section 23.1 Table 8.
    #[error("unbound parameter: ${name}")]
    #[diagnostic(code(SLENE_X_22G03))]
    UnboundParameter {
        /// Parameter name without the leading `$`.
        name: IStr,
        /// Source span requiring the parameter.
        #[label("unbound parameter")]
        span: SourceSpan,
    },

    /// A bound `$name` parameter had the wrong type for its runtime position.
    ///
    /// Maps to GQLSTATUS 22G03 per ISO/IEC 39075:2024 section 23.1 Table 8.
    #[error("invalid parameter type for ${name}: expected {expected}, got {actual}")]
    #[diagnostic(code(SLENE_X_22G03))]
    InvalidParameterType {
        /// Parameter name without the leading `$`.
        name: IStr,
        /// Human-readable expected type.
        expected: &'static str,
        /// Human-readable actual type.
        actual: &'static str,
        /// Source span requiring the parameter.
        #[label("invalid parameter type")]
        span: SourceSpan,
    },

    /// Scalar function name was not registered in the v1.1 closed set.
    ///
    /// Maps to GQLSTATUS 22G03 per ISO/IEC 39075:2024 section 23.1 Table 8.
    #[error("unknown function: {name}")]
    #[diagnostic(code(SLENE_X_22G03))]
    UnknownFunction {
        /// Function name as written by the caller.
        name: String,
        /// Source span for the function call.
        #[label("unknown function")]
        span: SourceSpan,
    },

    /// Scalar function received the wrong number of arguments.
    ///
    /// Maps to GQLSTATUS 22G03 per ISO/IEC 39075:2024 section 23.1 Table 8.
    #[error("function {name} expected {expected} argument(s), got {actual}")]
    #[diagnostic(code(SLENE_X_22G03))]
    FunctionArityMismatch {
        /// Function name as written by the caller.
        name: String,
        /// Human-readable arity contract.
        expected: &'static str,
        /// Actual argument count.
        actual: usize,
        /// Source span for the function call.
        #[label("wrong arity")]
        span: SourceSpan,
    },

    /// Scalar function call used an aggregate-only modifier.
    ///
    /// Maps to GQLSTATUS 22G03 per ISO/IEC 39075:2024 section 23.1 Table 8.
    #[error("function {name} does not allow {modifier}")]
    #[diagnostic(code(SLENE_X_22G03))]
    InvalidFunctionModifier {
        /// Function name as written by the caller.
        name: String,
        /// Rejected modifier.
        modifier: &'static str,
        /// Source span for the function call.
        #[label("invalid function modifier")]
        span: SourceSpan,
    },

    /// Expression feature is intentionally outside the v1.1 evaluator surface.
    ///
    /// Maps to GQLSTATUS 42N01, a selene-db implementation-defined subclass
    /// under standard class 42 per ISO/IEC 39075:2024 section 23.1.
    #[error("feature not supported in v1.1: {feature}")]
    #[diagnostic(code(SLENE_X_42N01))]
    FeatureNotInV1_1 {
        /// Stable feature tag.
        feature: &'static str,
        /// Source span requiring the feature.
        #[label("feature not supported")]
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
    #[diagnostic(code(SLENE_X_25N02))]
    InFailedTransaction {
        /// Source span for the rejected statement.
        #[label("aborted transaction; issue ROLLBACK to recover")]
        span: SourceSpan,
    },

    /// Caller-requested cooperative cancellation interrupted the statement.
    #[error("statement cancelled")]
    #[diagnostic(code(SLENE_X_5GQL2))]
    Cancelled {
        /// Source span for the cancellation checkpoint.
        #[label("cancelled here")]
        span: SourceSpan,
    },

    /// The statement deadline elapsed before completion.
    #[error("statement deadline exceeded")]
    #[diagnostic(code(SLENE_X_5GQL3))]
    Timeout {
        /// Deadline configured on the session.
        deadline: Instant,
        /// Duration since the deadline elapsed.
        elapsed: Duration,
        /// Source span for the timeout checkpoint.
        #[label("deadline exceeded here")]
        span: SourceSpan,
    },

    /// The statement produced more outermost result rows than allowed.
    #[error("statement row cap exceeded ({cap})")]
    #[diagnostic(code(SLENE_X_5GQL1))]
    RowCapExceeded {
        /// Maximum allowed outermost result rows.
        cap: usize,
        /// Source span for the result boundary.
        #[label("row cap exceeded here")]
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
            Self::Parse { source } => source.gqlstatus(),
            Self::Analysis { source } => source.gqlstatus(),
            Self::Plan { source } => source.gqlstatus(),
            Self::DataException { .. } => GqlStatus::DATA_EXCEPTION,
            Self::InvalidReference { .. } => GqlStatus::INVALID_REFERENCE,
            Self::UnboundParameter { .. } | Self::InvalidParameterType { .. } => {
                GqlStatus::INVALID_PROCEDURE_ARGUMENT
            }
            Self::UnknownFunction { .. }
            | Self::FunctionArityMismatch { .. }
            | Self::InvalidFunctionModifier { .. } => GqlStatus::DATATYPE_MISMATCH,
            Self::FeatureNotInV1_1 { .. } => GqlStatus::FEATURE_NOT_SUPPORTED,
            Self::InvalidTransactionState { .. }
            | Self::TransactionAlreadyActive { .. }
            | Self::NoActiveTransaction { .. } => GqlStatus::INVALID_TRANSACTION_STATE,
            Self::InFailedTransaction { .. } => GqlStatus::IN_FAILED_TRANSACTION,
            Self::Cancelled { .. } => GqlStatus::OPERATION_CANCELLED,
            Self::Timeout { .. } => GqlStatus::DEADLINE_EXCEEDED,
            Self::RowCapExceeded { .. } => GqlStatus::PROGRAM_LIMIT_EXCEEDED,
            Self::GraphMutation { .. } | Self::Flush { .. } => {
                GqlStatus::IMPLEMENTATION_DEFINED_ERROR
            }
            Self::Procedure { source, .. } => source.gqlstatus(),
            Self::ImplementationDefined { .. } => GqlStatus::IMPLEMENTATION_DEFINED_ERROR,
        }
    }
}
