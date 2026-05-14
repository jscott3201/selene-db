//! Provider lookup helpers for vector adapters.

use std::sync::Arc;

use selene_core::intern;
use selene_gql::{GraphContext, MutationContext, ProcedureError};
use selene_graph::ProviderTag;
use selene_vector::{HnswProvider, IvfProvider};

use crate::error::invalid_argument;

pub(crate) const VECT_TAG: ProviderTag = ProviderTag(*b"VECT");
pub(crate) const IVFP_TAG: ProviderTag = ProviderTag(*b"IVFP");

pub(crate) const HNSW_PROVIDER_NAME: &str = "selene-vector";
pub(crate) const IVF_PROVIDER_NAME: &str = "selene-vector-ivf";

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

pub(crate) fn with_ivf_provider_mut<R>(
    ctx: &MutationContext<'_, '_>,
    procedure: &'static str,
    f: impl FnOnce(&IvfProvider) -> Result<R, ProcedureError>,
) -> Result<R, ProcedureError> {
    let provider = ctx.index_provider_by_tag(IVFP_TAG).ok_or_else(|| {
        invalid_argument(format!("{procedure}: no IVFP index provider registered"))
    })?;
    let ivf = provider
        .as_any()
        .downcast_ref::<IvfProvider>()
        .ok_or_else(|| {
            invalid_argument(format!("{procedure}: IVFP provider is not an IVF provider"))
        })?;
    f(ivf)
}

pub(crate) fn emit_payload_bytes(
    ctx: &mut MutationContext<'_, '_>,
    procedure: &'static str,
    provider_name: &'static str,
    bytes: Vec<u8>,
) -> Result<(), ProcedureError> {
    let provider = intern(provider_name).map_err(|_| ProcedureError::Internal {
        detail: format!("{procedure}: provider name interner capacity exhausted"),
    })?;
    ctx.mutator()
        .extension_event(provider, Arc::from(bytes.into_boxed_slice()));
    Ok(())
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
