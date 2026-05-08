//! SharedGraph recovery helpers backed by selene-persist.

use std::path::Path;
use std::sync::Arc;

use selene_core::GraphId;
use selene_persist::{ProviderRegistry, RecoveryProvider};

use crate::core_provider::CoreProvider;
use crate::{GraphResult, SeleneGraph, SharedGraph};

impl SharedGraph {
    /// Recover a shared graph from a persistence directory.
    ///
    /// `graph_id` is the caller-asserted identity for the graph being
    /// recovered. If a snapshot is present, its `CORE/META` graph_id must
    /// agree with `graph_id`; if it disagrees, recovery fails so a misrouted
    /// directory cannot silently reconstruct under the wrong identity. If no
    /// snapshot exists (WAL-only or empty-directory recovery), `graph_id` is
    /// used directly.
    ///
    /// This helper registers only the built-in CORE provider. Embedders that
    /// need extension providers should call [`selene_persist::recover`] with
    /// their own registry and rebuild shared graph state explicitly.
    ///
    /// # Errors
    ///
    /// Returns persistence errors from snapshot/WAL recovery, [`crate::GraphError::Provider`]
    /// when a snapshot's META graph_id disagrees with `graph_id`, or graph
    /// errors if the recovered primary state cannot be materialized.
    pub fn recover(dir: &Path, graph_id: GraphId) -> GraphResult<Self> {
        let core = CoreProvider::new_for_recovery();
        let mut registry = ProviderRegistry::new();
        let provider: Arc<dyn RecoveryProvider> = core.clone();
        registry.register(provider)?;
        let outcome = selene_persist::recover(dir, &registry)?;
        let mut graph = core.finish_recovery(graph_id)?;
        // The committed graph generation must reflect every change that was
        // replayed. Snapshot+WAL recovery applies WAL entries past the
        // snapshot's sequence; without this bump, the next mutation would
        // increment from a stale snapshot generation, regressing or
        // duplicating sequencing relative to the recovered tip.
        graph.meta.generation = graph.meta.generation.max(outcome.last_wal_seq);
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
