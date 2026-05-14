//! Error mapping helpers for adapter procedures.

use selene_algorithms::{AlgorithmsError, TopoSortError};
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
