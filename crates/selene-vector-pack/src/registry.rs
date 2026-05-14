//! Vector-pack registration helpers.

use std::sync::Arc;

use selene_pack::{
    ExternalGraphProcedure, ExternalProcedurePack, ProcedurePackRegistry, RegistryError,
};

use crate::{search, state::VectorPackState};

/// Static external pack name registered with `selene-pack`.
pub const VECTOR_PACK_NAME: &str = "vector";

/// Canonical procedure names registered by the vector pack.
pub const VECTOR_PROCEDURE_NAMES: [&[&str]; 1] = [&["vector", "search"]];

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

    /// Return the external-pack description for registry-builder admission.
    #[must_use]
    pub fn external_pack(&self) -> ExternalProcedurePack {
        ExternalProcedurePack::new(VECTOR_PACK_NAME, self.procedures())
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
        vec![search::procedure(Arc::clone(&self.state))]
    }
}
