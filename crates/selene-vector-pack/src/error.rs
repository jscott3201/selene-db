//! Error helpers for vector procedure adapters.

use selene_core::{CancellationCause, CancellationChecker};
use selene_gql::ProcedureError;
use selene_vector::VectorError;

const VECTOR_CANCEL_CHECK_STRIDE: usize = 1024;

pub(crate) fn invalid_argument(detail: impl Into<String>) -> ProcedureError {
    ProcedureError::InvalidArgument {
        detail: detail.into(),
    }
}

#[inline(always)]
pub(crate) fn check_cancellation(checker: CancellationChecker<'_>) -> Result<(), ProcedureError> {
    if checker.is_disabled() {
        return Ok(());
    }
    checker.check().map_err(cancellation_error)
}

#[inline(always)]
pub(crate) fn check_cancellation_stride(
    checker: CancellationChecker<'_>,
    units_since_check: &mut usize,
) -> Result<(), ProcedureError> {
    if checker.is_disabled() {
        return Ok(());
    }
    *units_since_check = units_since_check.saturating_add(1);
    if *units_since_check >= VECTOR_CANCEL_CHECK_STRIDE {
        check_cancellation(checker)?;
        *units_since_check = 0;
    }
    Ok(())
}

#[inline]
fn cancellation_error(cause: CancellationCause) -> ProcedureError {
    match cause {
        CancellationCause::Cancelled => ProcedureError::Cancelled,
        CancellationCause::Timeout { elapsed } => ProcedureError::Timeout { elapsed },
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
