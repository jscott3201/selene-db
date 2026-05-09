//! Planner diagnostics.

use crate::{GqlStatus, SourceSpan, analyze::BindingId};

/// Query-planning failure.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum PlannerError {
    /// The planner reached a syntactic surface intentionally deferred beyond
    /// BRIEF-26.
    #[error("planner cannot lower {feature}: not implemented in this brief")]
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
}

impl PlannerError {
    /// Map this planner failure to its GQLSTATUS code.
    #[must_use]
    pub const fn gqlstatus(&self) -> GqlStatus {
        match self {
            Self::NotImplemented { .. }
            | Self::BindingResolutionLost { .. }
            | Self::ExpressionTypeMissing { .. } => GqlStatus::IMPLEMENTATION_DEFINED_ERROR,
        }
    }
}
