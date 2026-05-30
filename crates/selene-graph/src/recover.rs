//! SharedGraph recovery helpers backed by selene-persist.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use selene_core::{Change, ChangeKindSet, GraphId};
use selene_persist::{
    AuditLog, DEFAULT_AUDIT_FILE_NAME, DEFAULT_WAL_FILE_NAME, ProviderRegistry, RecoveryError,
    RecoveryProvider, RecoveryResult, WalConfig, WalWriter,
};

use crate::change_subscriber::ChangeSubscriber;
use crate::core_provider::CoreProvider;
use crate::graph_types::GraphTypeDef;
use crate::index_provider::{IndexProvider, ProviderError, ProviderTag, SubTag};
use crate::shared::{
    subscriber_tag_checked, validate_unique_provider_tags, validate_unique_subscriber_tags,
};
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
        Self::recover_inner(dir, graph_id, None, Vec::new(), Vec::new())
    }

    /// Recover an open (GG01) shared graph with extension index providers.
    ///
    /// Each supplied provider participates in snapshot-section recovery and WAL
    /// replay, then remains attached to the returned graph so future commits
    /// route to the same provider set. Subscribers whose tags match a provider
    /// receive filtered WAL changes during replay and remain attached for
    /// runtime commits.
    ///
    /// # Errors
    ///
    /// Returns persistence errors, [`crate::GraphError::Provider`] when provider
    /// tags are duplicated, subscriber tags are duplicated or unmatched, or
    /// recovered provider state is inconsistent, or graph errors when the
    /// recovered state cannot be materialized.
    pub fn recover_with_providers(
        dir: &Path,
        graph_id: GraphId,
        providers: Vec<Arc<dyn IndexProvider>>,
        subscribers: Vec<Arc<dyn ChangeSubscriber>>,
    ) -> GraphResult<Self> {
        Self::recover_inner(dir, graph_id, None, providers, subscribers)
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
        Self::recover_inner(
            dir,
            graph_id,
            Some(Arc::new(bound_type)),
            Vec::new(),
            Vec::new(),
        )
    }

    /// Recover a closed (GG02) shared graph with extension index providers.
    ///
    /// This is the provider-aware counterpart to [`SharedGraph::recover_closed`].
    /// Recovery validates the caller-supplied `bound_type`, rebuilds the supplied
    /// index providers from snapshot sections and WAL replay, fans matching
    /// subscribers during WAL replay, and returns a live WAL-backed graph with
    /// those providers and subscribers still attached.
    ///
    /// # Errors
    ///
    /// Returns persistence errors, [`crate::GraphError::Provider`] on provider
    /// tag duplication, subscriber tag duplication or mismatch, or type drift,
    /// [`crate::GraphError::TypeViolation`] when recovered entities don't
    /// conform to `bound_type`, or graph errors when the recovered state cannot
    /// be materialized.
    pub fn recover_closed_with_providers(
        dir: &Path,
        graph_id: GraphId,
        bound_type: GraphTypeDef,
        providers: Vec<Arc<dyn IndexProvider>>,
        subscribers: Vec<Arc<dyn ChangeSubscriber>>,
    ) -> GraphResult<Self> {
        Self::recover_inner(
            dir,
            graph_id,
            Some(Arc::new(bound_type)),
            providers,
            subscribers,
        )
    }

    fn recover_inner(
        dir: &Path,
        graph_id: GraphId,
        expected_bound_type: Option<Arc<GraphTypeDef>>,
        providers: Vec<Arc<dyn IndexProvider>>,
        subscribers: Vec<Arc<dyn ChangeSubscriber>>,
    ) -> GraphResult<Self> {
        let core = CoreProvider::new_for_recovery();
        validate_recovery_provider_tags(&core, &providers)?;
        validate_unique_subscriber_tags(&subscribers)?;
        let mut registry = ProviderRegistry::new();
        let truncate_expansion = core
            .truncate_expansion_handle()
            .expect("recovery-mode CoreProvider exposes a truncate-expansion handle");
        let provider: Arc<dyn RecoveryProvider> = core.clone();
        registry.register(provider)?;
        register_combined_recovery_providers(
            &mut registry,
            &providers,
            &subscribers,
            &truncate_expansion,
        )?;
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
        let writer = WalWriter::open(&dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default())?;
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
            subscribers,
            Vec::new(),
            Some(writer),
            audit_log,
        )
    }
}

struct IndexAndChangeRecoveryProvider {
    provider: Arc<dyn IndexProvider>,
    subscribers: Vec<Arc<dyn ChangeSubscriber>>,
    /// Shared handle to CORE's per-WAL-entry truncate-expansion buffer.
    ///
    /// BRIEF-150 / audit Item 11. CORE replays first (tag-sorted) and stages the
    /// per-row tombstones a declarative truncate expands to; this wrapper reads
    /// them so subscribers receive the same `NodeDeleted`/`EdgeDeleted` multiset
    /// the runtime path produces, never the declarative truncate variant.
    truncate_expansion: Arc<parking_lot::Mutex<Vec<Change>>>,
}

impl RecoveryProvider for IndexAndChangeRecoveryProvider {
    fn provider_tag(&self) -> [u8; 4] {
        self.provider.provider_tag().0
    }

    fn read_section(&self, sub: [u8; 4], bytes: &[u8]) -> RecoveryResult<()> {
        self.provider
            .read_section(SubTag(sub), bytes)
            .map_err(recovery_error)
    }

    fn on_change(&self, change: &Change) -> RecoveryResult<()> {
        self.on_changes(std::slice::from_ref(change))
    }

    fn on_changes(&self, changes: &[Change]) -> RecoveryResult<()> {
        for change in changes {
            self.provider.on_change(change).map_err(recovery_error)?;
        }
        if self.subscribers.is_empty() {
            return Ok(());
        }
        // Substitute each declarative truncate change with the per-row
        // tombstones CORE staged for this entry (CORE ran first), so subscribers
        // observe the identical effect of the runtime path and never see the
        // declarative variant they could not expand. Non-truncate commits use
        // the raw changes with zero extra allocation.
        let fanout = self.expanded_fanout(changes);
        let fanout_changes: &[Change] = fanout.as_deref().unwrap_or(changes);
        for subscriber in &self.subscribers {
            let (tag, kinds) = recovery_subscriber_filter(subscriber)?;
            for change in fanout_changes {
                if kinds.contains(change.kind()) {
                    recovery_subscriber_on_change(subscriber, tag, change)?;
                }
            }
        }
        Ok(())
    }
}

impl IndexAndChangeRecoveryProvider {
    /// Build a subscriber fan-out view that replaces declarative truncate
    /// changes with CORE's staged per-row tombstones.
    ///
    /// Returns `None` when no truncate change is present (the common path), so
    /// callers fan out the raw entry changes directly.
    fn expanded_fanout(&self, changes: &[Change]) -> Option<Vec<Change>> {
        if !changes.iter().any(|change| {
            matches!(
                change,
                Change::NodesOfTypeTruncated { .. }
                    | Change::EdgesOfTypeTruncated { .. }
                    | Change::GraphReset { .. }
            )
        }) {
            return None;
        }
        // CORE re-derives one combined tombstone list for every truncate in the
        // entry. Emit the non-truncate changes positionally and append the
        // staged tombstones in place of the truncate changes. Because truncate
        // tombstones are pure NodeDeleted/EdgeDeleted (order-independent for
        // derived-state delete_node/delete_edge subscribers), splicing the whole staged
        // list once preserves the per-row coverage invariant.
        let staged = self.truncate_expansion.lock().clone();
        let mut view = Vec::with_capacity(changes.len() + staged.len());
        let mut staged_emitted = false;
        for change in changes {
            match change {
                Change::NodesOfTypeTruncated { .. }
                | Change::EdgesOfTypeTruncated { .. }
                | Change::GraphReset { .. } => {
                    if !staged_emitted {
                        view.extend(staged.iter().cloned());
                        staged_emitted = true;
                    }
                }
                other => view.push(other.clone()),
            }
        }
        Some(view)
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

fn register_combined_recovery_providers(
    registry: &mut ProviderRegistry,
    providers: &[Arc<dyn IndexProvider>],
    subscribers: &[Arc<dyn ChangeSubscriber>],
    truncate_expansion: &Arc<parking_lot::Mutex<Vec<Change>>>,
) -> GraphResult<()> {
    let mut subscribers_by_tag = BTreeMap::<ProviderTag, Vec<Arc<dyn ChangeSubscriber>>>::new();
    for subscriber in subscribers {
        let tag = subscriber_tag_checked(subscriber, "change subscriber recovery setup")?;
        subscribers_by_tag
            .entry(tag)
            .or_default()
            .push(Arc::clone(subscriber));
    }

    for provider in providers {
        let subscribers = subscribers_by_tag
            .remove(&provider.provider_tag())
            .unwrap_or_default();
        let recovery_provider: Arc<dyn RecoveryProvider> =
            Arc::new(IndexAndChangeRecoveryProvider {
                provider: Arc::clone(provider),
                subscribers,
                truncate_expansion: Arc::clone(truncate_expansion),
            });
        registry.register(recovery_provider)?;
    }
    if let Some((tag, _)) = subscribers_by_tag.into_iter().next() {
        return Err(crate::GraphError::Provider(ProviderError::Inconsistent {
            reason: format!("change subscriber tag {tag} has no matching provider"),
        }));
    }
    Ok(())
}

fn recovery_error(error: ProviderError) -> RecoveryError {
    Box::new(error)
}

fn recovery_subscriber_filter(
    subscriber: &Arc<dyn ChangeSubscriber>,
) -> RecoveryResult<(ProviderTag, ChangeKindSet)> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        (subscriber.subscriber_tag(), subscriber.change_kinds())
    }))
    .map_err(|payload| {
        recovery_error(ProviderError::Inconsistent {
            reason: format!(
                "change subscriber tag/filter lookup panicked during recovery replay: {}",
                crate::panic_payload::describe(&payload)
            ),
        })
    })
}

fn recovery_subscriber_on_change(
    subscriber: &Arc<dyn ChangeSubscriber>,
    tag: ProviderTag,
    change: &Change,
) -> RecoveryResult<()> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        subscriber.on_change(change)
    })) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(recovery_error(error)),
        Err(payload) => Err(recovery_error(ProviderError::Inconsistent {
            reason: format!(
                "change subscriber {tag} on_change panicked during recovery replay: {}",
                crate::panic_payload::describe(&payload)
            ),
        })),
    }
}

#[cfg(test)]
#[path = "recover_tests.rs"]
mod tests;
