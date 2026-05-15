//! Shared parser for nullable GQL parallelism arguments.

use std::num::NonZeroUsize;

use selene_algorithms::Parallelism;
use selene_gql::{ProcedureError, Value};

use crate::error::invalid_argument;

/// Adapter-side upper bound for explicit Rayon thread counts.
pub(crate) const MAX_PARALLELISM_THREADS: usize = 1024;

/// Parse the common `parallelism` argument encoding.
pub(crate) fn parse_parallelism(
    procedure: &'static str,
    value: &Value,
) -> Result<Parallelism, ProcedureError> {
    match value {
        Value::Null => Ok(Parallelism::Auto),
        Value::Int(0) => Ok(Parallelism::Sequential),
        Value::Int(value) if *value > 0 => {
            let threads = usize::try_from(*value)
                .map_err(|_| invalid_argument(format!("{procedure}: parallelism is too large")))?;
            threads_parallelism(procedure, threads)
        }
        Value::Int(_) => Err(invalid_argument(format!(
            "{procedure}: parallelism must be NULL, 0, or a positive thread count"
        ))),
        Value::Uint(0) => Ok(Parallelism::Sequential),
        Value::Uint(value) => {
            let threads = usize::try_from(*value)
                .map_err(|_| invalid_argument(format!("{procedure}: parallelism is too large")))?;
            threads_parallelism(procedure, threads)
        }
        other => Err(invalid_argument(format!(
            "{procedure}: expected parallelism to be INTEGER or NULL, got {other:?}"
        ))),
    }
}

fn threads_parallelism(
    procedure: &'static str,
    threads: usize,
) -> Result<Parallelism, ProcedureError> {
    if threads > MAX_PARALLELISM_THREADS {
        return Err(invalid_argument(format!(
            "{procedure}: parallelism exceeds adapter-side cap of {MAX_PARALLELISM_THREADS} threads"
        )));
    }
    Ok(Parallelism::Threads(
        NonZeroUsize::new(threads).expect("positive thread count"),
    ))
}
