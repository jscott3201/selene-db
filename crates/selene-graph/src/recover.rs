//! SharedGraph recovery helpers backed by selene-persist.

use std::path::Path;
use std::sync::Arc;

use selene_core::GraphId;
use selene_persist::{DEFAULT_WAL_FILE_NAME, ProviderRegistry, RecoveryProvider, WalConfig};

use crate::core_provider::CoreProvider;
use crate::graph_types::GraphTypeDef;
use crate::{GraphResult, SharedGraph};

impl SharedGraph {
    /// Recover an open (GG01) shared graph from a persistence directory.
    ///
    /// `graph_id` is the caller-asserted identity. If a snapshot is present
    /// and declares a `bound_type`, recovery fails — a closed graph must be
    /// recovered via [`SharedGraph::recover_closed`].
    ///
    /// # Errors
    ///
    /// Returns persistence errors, [`crate::GraphError::Provider`] when a
    /// snapshot disagrees with `graph_id` or declares a closed binding, or
    /// graph errors when the recovered state cannot be materialized.
    pub fn recover(dir: &Path, graph_id: GraphId) -> GraphResult<Self> {
        Self::recover_inner(dir, graph_id, None)
    }

    /// Recover a closed (GG02) shared graph bound to `bound_type`.
    ///
    /// `graph_id` and `bound_type` are caller-asserted; recovery validates
    /// against the snapshot:
    ///
    /// - If the snapshot's `CORE/META` references a `bound_type` via
    ///   `CORE/GTYP`, it must equal `bound_type` or recovery fails (drift).
    /// - If the snapshot declares no binding but `bound_type` is provided,
    ///   recovery fails (snapshot says open, caller says closed).
    /// - If no snapshot is present (WAL-only or empty-dir), the caller's
    ///   `bound_type` is used and validation runs against replayed state.
    ///
    /// # Errors
    ///
    /// Returns persistence errors, [`crate::GraphError::Provider`] on type
    /// drift / inconsistency, [`crate::GraphError::TypeViolation`] when
    /// recovered entities don't conform to `bound_type`, or graph errors
    /// when the recovered state cannot be materialized.
    pub fn recover_closed(
        dir: &Path,
        graph_id: GraphId,
        bound_type: GraphTypeDef,
    ) -> GraphResult<Self> {
        Self::recover_inner(dir, graph_id, Some(Arc::new(bound_type)))
    }

    fn recover_inner(
        dir: &Path,
        graph_id: GraphId,
        expected_bound_type: Option<Arc<GraphTypeDef>>,
    ) -> GraphResult<Self> {
        let core = CoreProvider::new_for_recovery();
        let mut registry = ProviderRegistry::new();
        let provider: Arc<dyn RecoveryProvider> = core.clone();
        registry.register(provider)?;
        let outcome = selene_persist::recover(dir, &registry)?;
        let mut graph = core.finish_recovery(graph_id, expected_bound_type)?;
        // The committed graph generation must reflect every change that was
        // replayed. Snapshot+WAL recovery applies WAL entries past the
        // snapshot's sequence; without this bump, the next mutation would
        // increment from a stale snapshot generation, regressing or
        // duplicating sequencing relative to the recovered tip.
        graph.meta.generation = graph.meta.generation.max(outcome.last_wal_seq);
        // Reopen the WAL file as a live writer so post-recovery commits
        // continue to append durably. Without this, recover() returns a
        // graph whose commits go to memory only — a crash after recovery
        // would lose every post-recovery change even though the feature
        // advertises live WAL durability.
        Self::from_graph_with_wal(graph, dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default())
    }
}

#[cfg(test)]
#[path = "recover_tests.rs"]
mod tests;
