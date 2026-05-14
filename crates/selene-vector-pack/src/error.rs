//! Error helpers for vector procedure adapters.

use selene_gql::ProcedureError;
use selene_vector::VectorError;

pub(crate) fn invalid_argument(detail: impl Into<String>) -> ProcedureError {
    ProcedureError::InvalidArgument {
        detail: detail.into(),
    }
}

pub(crate) fn vector_error(error: VectorError) -> ProcedureError {
    match error {
        VectorError::DimensionsLocked { expected, observed }
        | VectorError::DimensionMismatch { expected, observed } => invalid_argument(format!(
            "vector.search query dimension mismatch: expected={expected} observed={observed}"
        )),
        VectorError::NonFiniteQueryComponent { .. } => {
            invalid_argument("vector.search query contains NaN or infinity")
        }
        VectorError::PqTrainingDeferred {
            observed_vectors,
            required,
        } => invalid_argument(format!(
            "vector.search PQ training deferred: observed={observed_vectors} required={required}"
        )),
        other => ProcedureError::Internal {
            detail: format!("vector.search provider error: {other}"),
        },
    }
}
