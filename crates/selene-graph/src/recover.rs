//! SharedGraph recovery helpers backed by selene-persist.

use std::path::Path;
use std::sync::Arc;

use selene_core::{Change, GraphId};
use selene_persist::{
    AuditLog, DEFAULT_AUDIT_FILE_NAME, DEFAULT_WAL_FILE_NAME, ProviderRegistry, RecoveryError,
    RecoveryProvider, RecoveryResult, WalConfig, WalWriter,
};

use crate::core_provider::CoreProvider;
use crate::graph_types::GraphTypeDef;
use crate::index_provider::{IndexProvider, ProviderError, SubTag};
use crate::shared::validate_unique_provider_tags;
use crate::{GraphError, GraphResult, SharedGraph};

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
        Self::recover_inner(dir, graph_id, None, Vec::new())
    }

    /// Recover an open (GG01) shared graph with extension index providers.
    ///
    /// Each supplied provider participates in snapshot-section recovery and WAL
    /// replay, then remains attached to the returned graph so future commits
    /// route to the same provider set.
    ///
    /// # Errors
    ///
    /// Returns persistence errors, [`crate::GraphError::Provider`] when provider
    /// tags are duplicated or recovered provider state is inconsistent, or graph
    /// errors when the recovered state cannot be materialized.
    pub fn recover_with_providers(
        dir: &Path,
        graph_id: GraphId,
        providers: Vec<Arc<dyn IndexProvider>>,
    ) -> GraphResult<Self> {
        Self::recover_inner(dir, graph_id, None, providers)
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
        Self::recover_inner(dir, graph_id, Some(Arc::new(bound_type)), Vec::new())
    }

    /// Recover a closed (GG02) shared graph with extension index providers.
    ///
    /// This is the provider-aware counterpart to [`SharedGraph::recover_closed`].
    /// Recovery validates the caller-supplied `bound_type`, rebuilds the supplied
    /// index providers from snapshot sections and WAL replay, and returns a live
    /// WAL-backed graph with those providers still attached.
    ///
    /// # Errors
    ///
    /// Returns persistence errors, [`crate::GraphError::Provider`] on provider
    /// tag duplication or type drift,
    /// [`crate::GraphError::TypeViolation`] when recovered entities don't
    /// conform to `bound_type`, or graph errors when the recovered state cannot
    /// be materialized.
    pub fn recover_closed_with_providers(
        dir: &Path,
        graph_id: GraphId,
        bound_type: GraphTypeDef,
        providers: Vec<Arc<dyn IndexProvider>>,
    ) -> GraphResult<Self> {
        Self::recover_inner(dir, graph_id, Some(Arc::new(bound_type)), providers)
    }

    fn recover_inner(
        dir: &Path,
        graph_id: GraphId,
        expected_bound_type: Option<Arc<GraphTypeDef>>,
        providers: Vec<Arc<dyn IndexProvider>>,
    ) -> GraphResult<Self> {
        // Bind recovery, live WAL reopen, and audit reopen to one physical
        // directory so a caller alias cannot retarget between those phases.
        let dir = std::fs::canonicalize(dir).map_err(selene_persist::PersistError::from)?;
        let core = CoreProvider::new_for_recovery();
        validate_recovery_provider_tags(&core, &providers)?;
        let mut registry = ProviderRegistry::new();
        let provider: Arc<dyn RecoveryProvider> = core.clone();
        registry.register(provider)?;
        register_index_recovery_providers(&mut registry, &providers)?;
        let outcome = selene_persist::recover(&dir, &registry)?;
        let mut graph = core.finish_recovery(graph_id, expected_bound_type)?;
        // Physical WAL sequences include checkpoint watermarks and can diverge
        // from logical graph generations. Advance the snapshot META generation
        // once per replayed logical commit (including ordinary empty commits),
        // never once per physical watermark or Change value.
        graph.meta.generation = graph
            .meta
            .generation
            .checked_add(outcome.wal_commit_entries_applied)
            .ok_or_else(|| GraphError::Inconsistent {
                reason: "graph generation overflow while applying recovered WAL commits".to_owned(),
            })?;
        mark_recovered_provider_generation(&providers, &graph, &outcome)?;
        #[cfg(test)]
        run_after_persist_recovery_hook();
        // Reopen the WAL file as a live writer so post-recovery commits
        // continue to append durably. Without this, recover() returns a
        // graph whose commits go to memory only — a crash after recovery
        // would lose every post-recovery change even though the feature
        // advertises live WAL durability.
        //
        // v1.2 BRIEF 2: the single committer is the sole fsync caller, so the
        // reopened WAL is driven in OnFlushOnly (the committer flushes once per
        // drained run). Recovery uses CommitBatching::Off below, so the committer
        // still fsyncs once per post-recovery commit — durability is unchanged.
        let writer = WalWriter::open(
            &dir.join(DEFAULT_WAL_FILE_NAME),
            WalConfig {
                sync_policy: selene_persist::SyncPolicy::OnFlushOnly,
                ..WalConfig::default()
            },
        )?;
        // Reattach the audit log iff one exists on disk (its `SLAU` presence
        // means the embedder enabled audit) so post-recovery lifecycle commits
        // keep being mirrored. The historical events already persist in the file
        // — recovery never re-derives them, and the WAL replay above does not
        // re-mirror (write_commit is live-only). Absent file → no audit.
        let audit_path = dir.join(DEFAULT_AUDIT_FILE_NAME);
        let audit_log = if audit_path.exists() {
            Some(AuditLog::open(&audit_path)?)
        } else {
            None
        };
        Self::from_graph_with_core_and_durables(
            graph,
            providers,
            Vec::new(),
            Some(writer),
            audit_log,
            // Recovery uses the BRIEF-1-equivalent policy: one fsync per commit.
            crate::committer_batch::CommitBatching::Off,
        )
    }
}

#[cfg(test)]
thread_local! {
    static AFTER_PERSIST_RECOVERY_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn set_after_persist_recovery_hook(hook: impl FnOnce() + 'static) {
    AFTER_PERSIST_RECOVERY_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn run_after_persist_recovery_hook() {
    AFTER_PERSIST_RECOVERY_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

/// Recovery wrapper that drives a runtime [`IndexProvider`] through
/// [`selene-persist`]'s `RecoveryProvider` interface during snapshot-section
/// reads and WAL replay.
struct IndexRecoveryProvider {
    provider: Arc<dyn IndexProvider>,
}

impl RecoveryProvider for IndexRecoveryProvider {
    fn provider_tag(&self) -> [u8; 4] {
        self.provider.provider_tag().0
    }

    fn read_section(&self, sub: [u8; 4], bytes: &[u8]) -> RecoveryResult<()> {
        self.provider
            .read_section(SubTag(sub), bytes)
            .map_err(recovery_error)
    }

    fn on_change(&self, change: &Change) -> RecoveryResult<()> {
        self.provider.on_change(change).map_err(recovery_error)
    }

    fn on_changes(&self, changes: &[Change]) -> RecoveryResult<()> {
        self.provider.on_changes(changes).map_err(recovery_error)
    }
}

fn validate_recovery_provider_tags(
    core: &Arc<CoreProvider>,
    providers: &[Arc<dyn IndexProvider>],
) -> GraphResult<()> {
    let mut all_providers = Vec::with_capacity(providers.len() + 1);
    let core_provider: Arc<dyn IndexProvider> = core.clone();
    all_providers.push(core_provider);
    all_providers.extend(providers.iter().cloned());
    validate_unique_provider_tags(&all_providers)
}

fn register_index_recovery_providers(
    registry: &mut ProviderRegistry,
    providers: &[Arc<dyn IndexProvider>],
) -> GraphResult<()> {
    for provider in providers {
        let recovery_provider: Arc<dyn RecoveryProvider> = Arc::new(IndexRecoveryProvider {
            provider: Arc::clone(provider),
        });
        registry.register(recovery_provider)?;
    }
    Ok(())
}

fn mark_recovered_provider_generation(
    providers: &[Arc<dyn IndexProvider>],
    graph: &crate::SeleneGraph,
    outcome: &selene_persist::RecoveryOutcome,
) -> GraphResult<()> {
    for provider in providers {
        let provider_tag = provider.provider_tag().0;
        let missing_snapshot_state = outcome.applied_snapshot_seq > 0
            && !provider.declared_sub_tags().is_empty()
            && outcome
                .snapshot_providers_invoked
                .binary_search(&provider_tag)
                .is_err();
        if missing_snapshot_state {
            provider.rebuild_from_graph(graph)?;
        }
        provider.on_commit_applied(graph.meta.generation)?;
    }
    Ok(())
}

fn recovery_error(error: ProviderError) -> RecoveryError {
    Box::new(error)
}

#[cfg(test)]
#[path = "recover_tests.rs"]
mod tests;
