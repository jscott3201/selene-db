//! Value-level parser for the nullable `parallelism` argument.
//!
//! Ported verbatim from the historical `selene-algorithms-pack/src/parallel.rs`
//! adapter. The thread-count *cap policy* now lives in `selene-algorithms`
//! ([`selene_algorithms::Parallelism::from_thread_count`] /
//! [`selene_algorithms::MAX_PARALLELISM_THREADS`]); this module keeps the
//! `Value`-level coercion (NULL→Auto, Int(0)/Uint(0)→Sequential, positive→thread
//! count, negative-int rejection, overflow check) and renders the same
//! procedure-qualified detail strings the pack adapter produced.

use std::num::NonZeroUsize;

use selene_algorithms::{MAX_PARALLELISM_THREADS, Parallelism};
use selene_core::Value;

use super::error::invalid_argument;
use crate::ProcedureError;

/// Parse the common `parallelism` argument encoding.
pub(super) fn parse_parallelism(
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
