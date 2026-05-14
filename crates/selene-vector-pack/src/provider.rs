//! Provider lookup helpers for vector adapters.

use selene_gql::{GraphContext, MutationContext, ProcedureError};
use selene_graph::ProviderTag;
use selene_vector::HnswProvider;

use crate::error::invalid_argument;

pub(crate) const VECT_TAG: ProviderTag = ProviderTag(*b"VECT");

pub(crate) fn with_hnsw_provider<R>(
    ctx: &GraphContext<'_>,
    procedure: &'static str,
    f: impl FnOnce(&HnswProvider) -> Result<R, ProcedureError>,
) -> Result<R, ProcedureError> {
    let provider = ctx.index_provider_by_tag(VECT_TAG).ok_or_else(|| {
        invalid_argument(format!("{procedure}: no VECT index provider registered"))
    })?;
    let hnsw = provider
        .as_any()
        .downcast_ref::<HnswProvider>()
        .ok_or_else(|| {
            invalid_argument(format!(
                "{procedure}: VECT provider is not an HNSW provider"
            ))
        })?;
    f(hnsw)
}

pub(crate) fn with_hnsw_provider_mut<R>(
    ctx: &MutationContext<'_, '_>,
    procedure: &'static str,
    f: impl FnOnce(&HnswProvider) -> Result<R, ProcedureError>,
) -> Result<R, ProcedureError> {
    let provider = ctx.index_provider_by_tag(VECT_TAG).ok_or_else(|| {
        invalid_argument(format!("{procedure}: no VECT index provider registered"))
    })?;
    let hnsw = provider
        .as_any()
        .downcast_ref::<HnswProvider>()
        .ok_or_else(|| {
            invalid_argument(format!(
                "{procedure}: VECT provider is not an HNSW provider"
            ))
        })?;
    f(hnsw)
}

pub(crate) fn reject_non_default_index(
    procedure: &'static str,
    index_name: &str,
) -> Result<(), ProcedureError> {
    if index_name == "default" {
        return Ok(());
    }
    Err(invalid_argument(format!(
        "{procedure}: unknown vector index '{index_name}'; v1.0 accepts only 'default'"
    )))
}
