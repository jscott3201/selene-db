//! Shared helpers for maintained candidate-state vector built-ins.

use selene_core::Value;
use selene_graph::{ProviderError, VectorCandidateSet};

use super::vector_common::{invalid_arg, string_arg};
use crate::procedure_registry::ProcedureError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CandidateStateOperation {
    Intersection,
    Union,
    StateDifference,
    CandidatesDifference,
}

impl CandidateStateOperation {
    pub(super) fn parse(proc_name: &'static str, raw: &str) -> Result<Self, ProcedureError> {
        match raw.to_ascii_lowercase().as_str() {
            "intersection" | "intersect" => Ok(Self::Intersection),
            "union" => Ok(Self::Union),
            "state_difference" | "state_minus_nodes" | "state_minus_candidates" => {
                Ok(Self::StateDifference)
            }
            "nodes_difference"
            | "nodes_minus_state"
            | "candidates_difference"
            | "candidates_minus_state"
            | "expanded_difference"
            | "expanded_minus_state" => Ok(Self::CandidatesDifference),
            _ => Err(invalid_arg(format!(
                "{proc_name} operation must be intersection, union, state_difference, or candidates_difference"
            ))),
        }
    }

    pub(super) fn compose(
        self,
        state: &VectorCandidateSet,
        candidates: &VectorCandidateSet,
    ) -> VectorCandidateSet {
        match self {
            Self::Intersection => state.intersection(candidates),
            Self::Union => state.union(candidates),
            Self::StateDifference => state.difference(candidates),
            Self::CandidatesDifference => candidates.difference(state),
        }
    }
}

pub(super) fn operation_arg(
    proc_name: &'static str,
    value: &Value,
) -> Result<CandidateStateOperation, ProcedureError> {
    let operation = string_arg(proc_name, value, "operation")?;
    CandidateStateOperation::parse(proc_name, operation.as_str())
}

pub(super) fn candidate_state_error(
    proc_name: &'static str,
    error: ProviderError,
) -> ProcedureError {
    ProcedureError::Internal {
        detail: format!("candidate-state provider error during {proc_name}: {error}"),
    }
}
