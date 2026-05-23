//! Planner diagnostics.

use selene_core::IStr;

use crate::{GqlStatus, SourceSpan, analyze::BindingId};

/// Query-planning failure.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum PlannerError {
    /// The planner reached a syntactic surface not supported in v1.0.
    #[error("planner cannot lower {feature}: not supported in v1.0")]
    #[diagnostic(code(SLENE_P_010))]
    NotImplemented {
        /// Stable missing-feature tag asserted by tests.
        feature: &'static str,
        /// Source span requiring the missing planner capability.
        #[label("not implemented by the planner yet")]
        span: SourceSpan,
    },

    /// A binding reference produced by the analyzer no longer points at a
    /// declaration during lowering.
    #[error("binding {binding:?} resolved by analyzer is missing during lowering")]
    #[diagnostic(code(SLENE_P_011))]
    BindingResolutionLost {
        /// Lost analyzer binding.
        binding: BindingId,
        /// Source span where the binding was required.
        #[label("binding should resolve here")]
        span: SourceSpan,
    },

    /// A value expression in the analyzed AST has no expression-type cell.
    #[error("expression cell missing for value expression at {span:?}")]
    #[diagnostic(code(SLENE_P_012))]
    ExpressionTypeMissing {
        /// Source span of the expression with no analyzer type cell.
        #[label("missing expression type")]
        span: SourceSpan,
    },

    /// The procedure registry passed to planning no longer contains a
    /// procedure that was resolved during analysis.
    #[error("procedure {procedure:?} not found in registry during planning")]
    #[diagnostic(code(SLENE_P_013))]
    UnknownProcedure {
        /// Qualified procedure name.
        procedure: Box<[IStr]>,
        /// Source span of the procedure call.
        #[label("unknown procedure")]
        span: SourceSpan,
    },

    /// A mutation statement reached the planner without its analyzer write set.
    #[error("mutation statement reached planner without analyzer write set")]
    #[diagnostic(code(SLENE_P_014))]
    WriteSetMissing {
        /// Source span of the mutation pipeline.
        #[label("missing write set")]
        span: SourceSpan,
    },

    /// Analyzer write-set order and planner AST walk disagreed.
    #[error("write-set entry could not be paired with an INSERT AST element")]
    #[diagnostic(code(SLENE_P_015))]
    WriteSetPatternMismatch {
        /// Source span of the unmatched write site.
        #[label("unmatched write-set entry")]
        span: SourceSpan,
    },

    /// Procedure registry metadata changed between analysis and planning.
    #[error("procedure {procedure:?} metadata changed between analyze and plan: {detail}")]
    #[diagnostic(code(SLENE_P_016))]
    ProcedureMetadataMismatch {
        /// Qualified procedure name.
        procedure: Box<[IStr]>,
        /// Stable mismatch detail tag.
        detail: &'static str,
        /// Source span of the procedure call or yield item.
        #[label("metadata mismatch")]
        span: SourceSpan,
    },

    /// The planner could not intern a static identifier because the process
    /// interner has reached its configured cap.
    #[error("planner could not intern static identifier during lowering: {detail}")]
    #[diagnostic(code(SLENE_P_017))]
    InternerCapExhausted {
        /// Stable static identifier detail tag.
        detail: &'static str,
        /// Source span of the construct requiring the static identifier.
        #[label("interner cap exhausted")]
        span: SourceSpan,
    },

    /// A planner-visible implementation-defined limit would be exceeded.
    #[error("{limit_name} {actual} exceeds implementation-defined limit {limit}")]
    #[diagnostic(code(SLENE_GQL_5GQL1))]
    ProgramLimitExceeded {
        /// Stable limit name asserted by tests.
        limit_name: &'static str,
        /// Configured limit.
        limit: u32,
        /// Requested value.
        actual: u32,
        /// Source span of the construct exceeding the limit.
        #[label("limit exceeded")]
        span: SourceSpan,
    },
}

impl PlannerError {
    /// Map this planner failure to its GQLSTATUS code.
    #[must_use]
    pub const fn gqlstatus(&self) -> GqlStatus {
        match self {
            Self::NotImplemented { .. } => GqlStatus::FEATURE_NOT_SUPPORTED,
            Self::BindingResolutionLost { .. }
            | Self::ExpressionTypeMissing { .. }
            | Self::UnknownProcedure { .. }
            | Self::WriteSetMissing { .. }
            | Self::WriteSetPatternMismatch { .. }
            | Self::ProcedureMetadataMismatch { .. }
            | Self::InternerCapExhausted { .. } => GqlStatus::IMPLEMENTATION_DEFINED_ERROR,
            Self::ProgramLimitExceeded { .. } => GqlStatus::PROGRAM_LIMIT_EXCEEDED,
        }
    }
}
