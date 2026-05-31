//! Error-mapping helpers for the native `algo.*` procedures.
//!
//! Ported verbatim from the historical procedure-pack error adapter so the
//! user-visible `ProcedureError` detail strings (and their GQLSTATUS mapping)
//! are byte-identical to the pack era. The native runners
//! call the underlying `selene_algorithms` `*_with_checker` functions directly,
//! so these mappers receive the raw algorithm error types and render the same
//! procedure-qualified messages.

use selene_algorithms::{AlgorithmAborted, AlgorithmsError, PathfindingError, TopoSortError};
use selene_core::CancellationCause;

use crate::ProcedureError;

pub(super) fn invalid_argument(detail: impl Into<String>) -> ProcedureError {
    ProcedureError::InvalidArgument {
        detail: detail.into(),
    }
}

pub(super) fn algorithm_error(error: AlgorithmsError) -> ProcedureError {
    invalid_argument(error.to_string())
}

pub(super) fn algorithm_aborted(error: AlgorithmAborted) -> ProcedureError {
    match error.cause {
        CancellationCause::Cancelled => ProcedureError::Cancelled,
        CancellationCause::Timeout { elapsed } => ProcedureError::Timeout { elapsed },
    }
}

pub(super) fn topo_sort_error(error: TopoSortError) -> ProcedureError {
    match error {
        TopoSortError::NotADag { .. } => {
            invalid_argument("algo.topological_sort: projection contains a directed cycle")
        }
        TopoSortError::Aborted { source } => algorithm_aborted(source),
        other => invalid_argument(other.to_string()),
    }
}

pub(super) fn pathfinding_error(
    procedure: &'static str,
    error: PathfindingError,
) -> ProcedureError {
    match error {
        PathfindingError::NegativeWeight { .. } => {
            invalid_argument(format!("{procedure}: traversed edge has negative weight"))
        }
        PathfindingError::NaNWeight { .. } => {
            invalid_argument(format!("{procedure}: traversed edge has NaN weight"))
        }
        PathfindingError::TooLarge { .. } => {
            invalid_argument("algo.apsp: projection node count exceeds max_nodes limit")
        }
        PathfindingError::Aborted { source } => algorithm_aborted(source),
        _other => invalid_argument(format!("{procedure}: pathfinding failed")),
    }
}
