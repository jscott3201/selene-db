//! Error mapping helpers for adapter procedures.

use selene_algorithms::{AlgorithmsError, PathfindingError, TopoSortError};
use selene_gql::ProcedureError;

pub(crate) fn invalid_argument(detail: impl Into<String>) -> ProcedureError {
    ProcedureError::InvalidArgument {
        detail: detail.into(),
    }
}

pub(crate) fn algorithm_error(error: AlgorithmsError) -> ProcedureError {
    invalid_argument(error.to_string())
}

pub(crate) fn topo_sort_error(error: TopoSortError) -> ProcedureError {
    match error {
        TopoSortError::NotADag { .. } => {
            invalid_argument("algo.topological_sort: projection contains a directed cycle")
        }
        other => invalid_argument(other.to_string()),
    }
}

pub(crate) fn pathfinding_error(
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
        _other => invalid_argument(format!("{procedure}: pathfinding failed")),
    }
}
