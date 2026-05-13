//! Algorithms-pack registration helpers.

use std::sync::Arc;

use selene_pack::{
    ExternalGraphProcedure, ExternalProcedurePack, ProcedurePackRegistry, RegistryError,
};

use crate::{pagerank, projection, state::AlgorithmsPackState};

/// Static external pack name registered with `selene-pack`.
pub const ALGORITHMS_PACK_NAME: &str = "selene-algorithms";

/// Canonical procedure names registered by the B1 algorithms pack.
pub const ALGO_PROCEDURE_NAMES: [&[&str]; 5] = [
    &["algo", "projection_build"],
    &["algo", "projection_get"],
    &["algo", "projection_drop"],
    &["algo", "projection_list"],
    &["algo", "pagerank"],
];

/// Construct-time handle for the algorithms procedure pack.
#[derive(Clone, Debug, Default)]
pub struct AlgorithmsPack {
    state: Arc<AlgorithmsPackState>,
}

impl AlgorithmsPack {
    /// Construct a fresh algorithms pack with empty graph-scoped catalogs.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the external-pack description for registry-builder admission.
    #[must_use]
    pub fn external_pack(&self) -> ExternalProcedurePack {
        ExternalProcedurePack::new(ALGORITHMS_PACK_NAME, self.procedures())
    }

    /// Construct a registry containing platform built-ins and algorithms pack.
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
        let mut procedures = projection::procedures(Arc::clone(&self.state));
        procedures.push(pagerank::procedure(Arc::clone(&self.state)));
        procedures
    }
}
