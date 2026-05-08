//! SharedGraph recovery helpers backed by selene-persist.

use std::path::Path;
use std::sync::Arc;

use selene_persist::{ProviderRegistry, RecoveryProvider};

use crate::core_provider::CoreProvider;
use crate::{GraphResult, SeleneGraph, SharedGraph};

impl SharedGraph {
    /// Recover a shared graph from a persistence directory.
    ///
    /// This helper registers only the built-in CORE provider. Embedders that
    /// need extension providers should call [`selene_persist::recover`] with
    /// their own registry and rebuild shared graph state explicitly.
    ///
    /// # Errors
    ///
    /// Returns persistence errors from snapshot/WAL recovery, or graph errors
    /// if the recovered primary state cannot be materialized.
    pub fn recover(dir: &Path) -> GraphResult<Self> {
        let core = CoreProvider::new_for_recovery();
        let mut registry = ProviderRegistry::new();
        let provider: Arc<dyn RecoveryProvider> = core.clone();
        registry.register(provider)?;
        let _outcome = selene_persist::recover(dir, &registry)?;
        let graph = core.finish_recovery()?;
        Self::from_recovered(graph)
    }

    /// Build shared graph state from recovered primary columns.
    ///
    /// Derived adjacency and secondary indexes are rebuilt from the canonical
    /// node/edge stores before publication.
    pub(crate) fn from_recovered(graph: SeleneGraph) -> GraphResult<Self> {
        Self::try_from_graph(graph)
    }
}

#[cfg(test)]
#[path = "recover_tests.rs"]
mod tests;
