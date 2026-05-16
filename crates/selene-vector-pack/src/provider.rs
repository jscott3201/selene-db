//! Provider lookup helpers for vector adapters.

use std::sync::Arc;

use selene_core::intern;
use selene_gql::{GraphContext, MutationContext, ProcedureError};
use selene_graph::{IndexProvider, ProviderTag};
use selene_vector::{
    Catalog, CreateIndexOutcome, DropIndexOutcome, HnswConfig, HnswIndexRegistry, HnswProvider,
    IndexListEntry, IvfConfig, IvfIndexRegistry, IvfProvider, VectorError, encode_named_payload,
};

use crate::error::invalid_argument;

pub(crate) const VECT_TAG: ProviderTag = ProviderTag(*b"VECT");
pub(crate) const IVFP_TAG: ProviderTag = ProviderTag(*b"IVFP");

pub(crate) const HNSW_PROVIDER_NAME: &str = "selene-vector";
pub(crate) const IVF_PROVIDER_NAME: &str = "selene-vector-ivf";
pub(crate) const LIFECYCLE_PROVIDER_NAME: &str = "selene-vector-lifecycle";

pub(crate) fn with_hnsw_provider<R>(
    ctx: &GraphContext<'_>,
    procedure: &'static str,
    index_name: &str,
    f: impl FnOnce(&HnswProvider) -> Result<R, ProcedureError>,
) -> Result<R, ProcedureError> {
    let provider = ctx.index_provider_by_tag(VECT_TAG).ok_or_else(|| {
        invalid_argument(format!("{procedure}: no VECT index provider registered"))
    })?;
    if let Some(registry) = provider.as_any().downcast_ref::<HnswIndexRegistry>() {
        let hnsw = registry.get(index_name).ok_or_else(|| {
            invalid_argument(format!(
                "{procedure}: VECT registry has no vector index '{index_name}'"
            ))
        })?;
        return f(hnsw.as_ref());
    }
    let hnsw = provider
        .as_any()
        .downcast_ref::<HnswProvider>()
        .ok_or_else(|| {
            invalid_argument(format!(
                "{procedure}: VECT provider is not an HNSW provider"
            ))
        })?;
    if index_name != "default" {
        return Err(invalid_argument(format!(
            "{procedure}: VECT registry has no vector index '{index_name}'"
        )));
    }
    f(hnsw)
}

pub(crate) fn with_hnsw_provider_mut<R>(
    ctx: &MutationContext<'_, '_>,
    procedure: &'static str,
    index_name: &str,
    f: impl FnOnce(&HnswProvider) -> Result<R, ProcedureError>,
) -> Result<R, ProcedureError> {
    let provider = ctx.index_provider_by_tag(VECT_TAG).ok_or_else(|| {
        invalid_argument(format!("{procedure}: no VECT index provider registered"))
    })?;
    if let Some(registry) = provider.as_any().downcast_ref::<HnswIndexRegistry>() {
        let hnsw = registry.get(index_name).ok_or_else(|| {
            invalid_argument(format!(
                "{procedure}: VECT registry has no vector index '{index_name}'"
            ))
        })?;
        return f(hnsw.as_ref());
    }
    let hnsw = provider
        .as_any()
        .downcast_ref::<HnswProvider>()
        .ok_or_else(|| {
            invalid_argument(format!(
                "{procedure}: VECT provider is not an HNSW provider"
            ))
        })?;
    if index_name != "default" {
        return Err(invalid_argument(format!(
            "{procedure}: VECT registry has no vector index '{index_name}'"
        )));
    }
    f(hnsw)
}

pub(crate) fn with_ivf_provider<R>(
    ctx: &GraphContext<'_>,
    procedure: &'static str,
    index_name: &str,
    f: impl FnOnce(&IvfProvider) -> Result<R, ProcedureError>,
) -> Result<R, ProcedureError> {
    let provider = ctx.index_provider_by_tag(IVFP_TAG).ok_or_else(|| {
        invalid_argument(format!("{procedure}: no IVFP index provider registered"))
    })?;
    if let Some(registry) = provider.as_any().downcast_ref::<IvfIndexRegistry>() {
        let ivf = registry.get(index_name).ok_or_else(|| {
            invalid_argument(format!(
                "{procedure}: IVFP registry has no vector index '{index_name}'"
            ))
        })?;
        return f(ivf.as_ref());
    }
    let ivf = provider
        .as_any()
        .downcast_ref::<IvfProvider>()
        .ok_or_else(|| {
            invalid_argument(format!("{procedure}: IVFP provider is not an IVF provider"))
        })?;
    if index_name != "default" {
        return Err(invalid_argument(format!(
            "{procedure}: IVFP registry has no vector index '{index_name}'"
        )));
    }
    f(ivf)
}

pub(crate) fn with_ivf_provider_mut<R>(
    ctx: &MutationContext<'_, '_>,
    procedure: &'static str,
    index_name: &str,
    f: impl FnOnce(&IvfProvider) -> Result<R, ProcedureError>,
) -> Result<R, ProcedureError> {
    let provider = ctx.index_provider_by_tag(IVFP_TAG).ok_or_else(|| {
        invalid_argument(format!("{procedure}: no IVFP index provider registered"))
    })?;
    if let Some(registry) = provider.as_any().downcast_ref::<IvfIndexRegistry>() {
        let ivf = registry.get(index_name).ok_or_else(|| {
            invalid_argument(format!(
                "{procedure}: IVFP registry has no vector index '{index_name}'"
            ))
        })?;
        return f(ivf.as_ref());
    }
    let ivf = provider
        .as_any()
        .downcast_ref::<IvfProvider>()
        .ok_or_else(|| {
            invalid_argument(format!("{procedure}: IVFP provider is not an IVF provider"))
        })?;
    if index_name != "default" {
        return Err(invalid_argument(format!(
            "{procedure}: IVFP registry has no vector index '{index_name}'"
        )));
    }
    f(ivf)
}

pub(crate) struct RegistryCatalog {
    hnsw_provider: Arc<dyn IndexProvider>,
    ivf_provider: Arc<dyn IndexProvider>,
}

impl RegistryCatalog {
    pub(crate) fn hnsw_default_config(&self) -> Result<HnswConfig, ProcedureError> {
        self.hnsw().default_config().map_err(registry_error)
    }

    pub(crate) fn ivf_default_config(&self) -> Result<IvfConfig, ProcedureError> {
        self.ivf().default_config().map_err(registry_error)
    }

    pub(crate) fn create_hnsw_index(
        &self,
        name: &str,
        config: HnswConfig,
    ) -> Result<CreateIndexOutcome, ProcedureError> {
        Catalog::create_hnsw_index_in(self.hnsw(), self.ivf(), name, config).map_err(registry_error)
    }

    pub(crate) fn create_ivf_index(
        &self,
        name: &str,
        config: IvfConfig,
    ) -> Result<CreateIndexOutcome, ProcedureError> {
        Catalog::create_ivf_index_in(self.hnsw(), self.ivf(), name, config).map_err(registry_error)
    }

    pub(crate) fn drop_index(&self, name: &str) -> Result<DropIndexOutcome, ProcedureError> {
        Catalog::drop_index_in(self.hnsw(), self.ivf(), name).map_err(registry_error)
    }

    pub(crate) fn list_indexes(&self) -> Vec<IndexListEntry> {
        Catalog::list_indexes_in(self.hnsw(), self.ivf())
    }

    fn hnsw(&self) -> &HnswIndexRegistry {
        self.hnsw_provider
            .as_any()
            .downcast_ref::<HnswIndexRegistry>()
            .expect("RegistryCatalog constructed from HNSW registry")
    }

    fn ivf(&self) -> &IvfIndexRegistry {
        self.ivf_provider
            .as_any()
            .downcast_ref::<IvfIndexRegistry>()
            .expect("RegistryCatalog constructed from IVF registry")
    }
}

pub(crate) fn catalog_from_graph(
    ctx: &GraphContext<'_>,
    procedure: &'static str,
) -> Result<RegistryCatalog, ProcedureError> {
    registry_catalog_from_providers(
        ctx.index_provider_by_tag(VECT_TAG),
        ctx.index_provider_by_tag(IVFP_TAG),
        procedure,
    )
}

pub(crate) fn catalog_from_mutation(
    ctx: &MutationContext<'_, '_>,
    procedure: &'static str,
) -> Result<RegistryCatalog, ProcedureError> {
    registry_catalog_from_providers(
        ctx.index_provider_by_tag(VECT_TAG),
        ctx.index_provider_by_tag(IVFP_TAG),
        procedure,
    )
}

fn registry_catalog_from_providers(
    hnsw_provider: Option<Arc<dyn IndexProvider>>,
    ivf_provider: Option<Arc<dyn IndexProvider>>,
    procedure: &'static str,
) -> Result<RegistryCatalog, ProcedureError> {
    let hnsw_provider = hnsw_provider.ok_or_else(|| {
        invalid_argument(format!("{procedure}: no VECT index provider registered"))
    })?;
    if hnsw_provider
        .as_any()
        .downcast_ref::<HnswIndexRegistry>()
        .is_none()
    {
        return Err(invalid_argument(format!(
            "{procedure}: VECT provider is not an HNSW registry"
        )));
    }
    let ivf_provider = ivf_provider.ok_or_else(|| {
        invalid_argument(format!("{procedure}: no IVFP index provider registered"))
    })?;
    if ivf_provider
        .as_any()
        .downcast_ref::<IvfIndexRegistry>()
        .is_none()
    {
        return Err(invalid_argument(format!(
            "{procedure}: IVFP provider is not an IVF registry"
        )));
    }
    Ok(RegistryCatalog {
        hnsw_provider,
        ivf_provider,
    })
}

pub(crate) fn emit_payload_bytes(
    ctx: &mut MutationContext<'_, '_>,
    procedure: &'static str,
    provider_name: &'static str,
    index_name: &str,
    bytes: Vec<u8>,
) -> Result<(), ProcedureError> {
    let bytes =
        encode_named_payload(index_name, bytes).map_err(|error| ProcedureError::Internal {
            detail: format!("{procedure}: named payload encode failed: {error}"),
        })?;
    let provider = intern(provider_name).map_err(|_| ProcedureError::Internal {
        detail: format!("{procedure}: provider name interner capacity exhausted"),
    })?;
    ctx.mutator()
        .extension_event(provider, Arc::from(bytes.into_boxed_slice()));
    Ok(())
}

fn registry_error(error: VectorError) -> ProcedureError {
    invalid_argument(format!("{error}"))
}
