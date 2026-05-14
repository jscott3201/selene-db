//! Error helpers for vector procedure adapters.

use selene_gql::ProcedureError;
use selene_vector::VectorError;

pub(crate) fn invalid_argument(detail: impl Into<String>) -> ProcedureError {
    ProcedureError::InvalidArgument {
        detail: detail.into(),
    }
}

pub(crate) fn vector_error(procedure: &'static str, error: VectorError) -> ProcedureError {
    match error {
        VectorError::DimensionsLocked { expected, observed }
        | VectorError::DimensionMismatch { expected, observed } => invalid_argument(format!(
            "{procedure}: query dimension mismatch: expected={expected} observed={observed}"
        )),
        VectorError::NonFiniteQueryComponent { .. } => {
            invalid_argument(format!("{procedure}: query contains NaN or infinity"))
        }
        VectorError::PqTrainingDeferred {
            observed_vectors,
            required,
        } => invalid_argument(format!(
            "{procedure}: PQ training deferred: observed={observed_vectors} required={required}"
        )),
        VectorError::IvfInvalidNProbe { n_probe, k_coarse } => invalid_argument(format!(
            "{procedure}: n_probe={n_probe} out of range (k_coarse={k_coarse})"
        )),
        other => ProcedureError::Internal {
            detail: format!("{procedure}: provider error: {other}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ivf_invalid_nprobe_maps_to_invalid_argument() {
        let err = vector_error(
            "vector.ivf_search",
            VectorError::IvfInvalidNProbe {
                n_probe: 0,
                k_coarse: 16,
            },
        );

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }
}
