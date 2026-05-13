//! Error mapping helpers for adapter procedures.

use selene_algorithms::AlgorithmsError;
use selene_gql::ProcedureError;

pub(crate) fn invalid_argument(detail: impl Into<String>) -> ProcedureError {
    ProcedureError::InvalidArgument {
        detail: detail.into(),
    }
}

pub(crate) fn algorithm_error(error: AlgorithmsError) -> ProcedureError {
    invalid_argument(error.to_string())
}
