//! Vector-pack registration helpers.

use std::sync::Arc;

use selene_pack::{
    ExternalGraphProcedure, ExternalMutationProcedure, ExternalProcedurePack,
    ProcedurePackRegistry, RegistryError,
};

use crate::{
    bulk_delete, bulk_upsert, create_index, delete, drop_index, ivf_bulk_delete, ivf_bulk_upsert,
    ivf_search, ivf_stats, list_indexes, search,
    state::{VectorPackConfig, VectorPackState},
    upsert,
};

/// Static external pack name registered with `selene-pack`.
pub const VECTOR_PACK_NAME: &str = "vector";

/// Canonical procedure names registered by the vector pack.
pub const VECTOR_PROCEDURE_NAMES: [&[&str]; 12] = [
    &["vector", "search"],
    &["vector", "upsert"],
    &["vector", "delete"],
    &["vector", "bulk_upsert"],
    &["vector", "bulk_delete"],
    &["vector", "create_index"],
    &["vector", "drop_index"],
    &["vector", "list_indexes"],
    &["vector", "ivf_bulk_upsert"],
    &["vector", "ivf_bulk_delete"],
    &["vector", "ivf_search"],
    &["vector", "ivf_stats"],
];

/// Construct-time handle for the vector procedure pack.
#[derive(Clone, Debug, Default)]
pub struct VectorPack {
    state: Arc<VectorPackState>,
}

impl VectorPack {
    /// Construct a fresh vector pack.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a vector pack with explicit procedure configuration.
    #[must_use]
    pub fn with_config(config: VectorPackConfig) -> Self {
        Self {
            state: Arc::new(VectorPackState::new(config)),
        }
    }

    /// Return this pack's shared configuration.
    #[must_use]
    pub fn config(&self) -> &VectorPackConfig {
        self.state.config()
    }

    /// Return the external-pack description for registry-builder admission.
    #[must_use]
    pub fn external_pack(&self) -> ExternalProcedurePack {
        ExternalProcedurePack::new(VECTOR_PACK_NAME, self.procedures())
            .with_mutation_procedures(self.mutation_procedures())
    }

    /// Construct a registry containing platform built-ins and the vector pack.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] if registration metadata is malformed or
    /// conflicts with another procedure.
    pub fn registry_with_builtins(&self) -> Result<ProcedurePackRegistry, RegistryError> {
        ProcedurePackRegistry::builder()
            .with_builtins()
            .with_external_pack(self.external_pack())
            .build()
    }

    fn procedures(&self) -> Vec<Arc<dyn ExternalGraphProcedure>> {
        vec![
            search::procedure(Arc::clone(&self.state)),
            list_indexes::procedure(Arc::clone(&self.state)),
            ivf_search::procedure(Arc::clone(&self.state)),
            ivf_stats::procedure(Arc::clone(&self.state)),
        ]
    }

    fn mutation_procedures(&self) -> Vec<Arc<dyn ExternalMutationProcedure>> {
        vec![
            upsert::procedure(Arc::clone(&self.state)),
            delete::procedure(Arc::clone(&self.state)),
            bulk_upsert::procedure(Arc::clone(&self.state)),
            bulk_delete::procedure(Arc::clone(&self.state)),
            create_index::procedure(Arc::clone(&self.state)),
            drop_index::procedure(Arc::clone(&self.state)),
            ivf_bulk_upsert::procedure(Arc::clone(&self.state)),
            ivf_bulk_delete::procedure(Arc::clone(&self.state)),
        ]
    }
}
