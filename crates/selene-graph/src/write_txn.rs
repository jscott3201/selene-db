//! Write transaction RAII handle per spec 03 sections 4 and 6.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

use arc_swap::ArcSwap;
use parking_lot::{MutexGuard, RwLockWriteGuard};
use selene_core::{Change, HlcTimestamp, Origin, metrics};

use crate::durable_provider::DurableProvider;
use crate::error::{GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::id_allocator::IdAllocator;
use crate::index_provider::IndexProvider;
use crate::mutator::Mutator;
use crate::type_validator::TypeWarning;

/// Non-fatal graph commit warning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitWarning {
    /// Closed-graph validation warning.
    pub warning: TypeWarning,
}

/// Result metadata returned after a successful commit.
#[derive(Clone, Debug, PartialEq)]
pub struct CommitOutcome {
    /// Published graph generation.
    pub generation: u64,
    /// Changes produced by the mutation funnel.
    pub changes: Vec<Change>,
    /// Opaque caller-supplied principal bytes for future WAL headers.
    pub principal: Option<Arc<[u8]>>,
    /// Highest durable sequence assigned by commit-critical providers.
    pub durable_at: Option<u64>,
    /// Next node ID after commit.
    pub next_node_id: u64,
    /// Next edge ID after commit.
    pub next_edge_id: u64,
    /// Non-fatal warnings produced during commit validation.
    pub warnings: Vec<CommitWarning>,
}

/// RAII owner of the single graph write lock.
pub struct WriteTxn<'g> {
    pub(crate) guard: RwLockWriteGuard<'g, Arc<SeleneGraph>>,
    pub(crate) snapshot: Arc<ArcSwap<SeleneGraph>>,
    pub(crate) schema_version: Arc<AtomicU64>,
    pub(crate) pre_txn: Option<Arc<SeleneGraph>>,
    pub(crate) allocator: MutexGuard<'g, IdAllocator>,
    pub(crate) providers: Vec<Arc<dyn IndexProvider>>,
    pub(crate) durable_providers: Vec<Arc<dyn DurableProvider>>,
    pub(crate) changes: Vec<Change>,
    pub(crate) warnings: Vec<CommitWarning>,
}

impl<'g> WriteTxn<'g> {
    pub(crate) fn new(
        guard: RwLockWriteGuard<'g, Arc<SeleneGraph>>,
        snapshot: Arc<ArcSwap<SeleneGraph>>,
        schema_version: Arc<AtomicU64>,
        allocator: MutexGuard<'g, IdAllocator>,
        providers: Vec<Arc<dyn IndexProvider>>,
        durable_providers: Vec<Arc<dyn DurableProvider>>,
    ) -> Self {
        let pre_txn = Some(Arc::clone(&*guard));
        Self {
            guard,
            snapshot,
            schema_version,
            pre_txn,
            allocator,
            providers,
            durable_providers,
            changes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Borrow a mutator tied to this transaction.
    #[must_use]
    pub fn mutator(&mut self) -> Mutator<'_, 'g> {
        Mutator::new(self, Origin::Local)
    }

    /// Borrow the transaction-local working graph.
    #[must_use]
    pub fn read(&self) -> &SeleneGraph {
        self.guard.as_ref()
    }

    pub(crate) fn guard_mut(&mut self) -> &mut SeleneGraph {
        Arc::make_mut(&mut *self.guard)
    }

    /// Commit without caller principal bytes.
    pub fn commit(self) -> GraphResult<CommitOutcome> {
        self.commit_with_principal(None)
    }

    /// Commit with optional caller-owned principal bytes for D12 audit replay.
    ///
    /// Registered index providers are notified after the new graph snapshot is
    /// published, **with the write lock and allocator mutex still held**, so
    /// that two concurrent commits cannot interleave their `on_change`
    /// callbacks (the per-graph serialization contract from Spec 06).
    /// Same-thread re-entrant
    /// provider calls into `SharedGraph::begin_write()` are detected via a
    /// thread-local fanout counter and panic with a clear message; the
    /// outer `std::panic::catch_unwind` in `notify_providers` catches those
    /// panics (along with provider-internal panics and returned errors) so
    /// a single misbehaving provider can never abort the writer thread.
    /// Cross-thread re-entry (a provider waiting on a spawned worker that
    /// calls `begin_write`) is documented misuse — see `reentry.rs` and
    /// the `IndexProvider` rustdoc.
    #[tracing::instrument(
        name = "selene.graph.commit",
        skip(self, principal),
        fields(change_count = self.change_count())
    )]
    pub fn commit_with_principal(
        mut self,
        principal: Option<Arc<[u8]>>,
    ) -> GraphResult<CommitOutcome> {
        let started = Instant::now();
        debug_assert!(
            self.pre_txn.is_some(),
            "pre_txn must be present at commit entry"
        );

        let schema_changed = self
            .changes
            .iter()
            .any(|change| matches!(change, Change::SchemaChanged { .. }));
        let next_node_id = self.allocator.peek_next_node();
        let next_edge_id = self.allocator.peek_next_edge();
        {
            let graph = self.guard_mut();
            graph.meta.generation = graph
                .meta
                .generation
                .checked_add(1)
                .expect("graph generation exhausted");
            graph.meta.next_node_id = next_node_id;
            graph.meta.next_edge_id = next_edge_id;
        }

        let generation = self.read().meta.generation;

        let mut validation_warnings = Vec::new();
        if let Some(type_def) = self.read().meta.bound_type.as_deref() {
            for change in &self.changes {
                validation_warnings.extend(
                    crate::type_validator::validate_change(change, self.read(), type_def)?
                        .into_iter()
                        .map(|warning| CommitWarning { warning }),
                );
            }
            if schema_changed {
                validation_warnings.extend(
                    crate::type_validator::validate_entity_state(self.read(), type_def)?
                        .into_iter()
                        .map(|warning| CommitWarning { warning }),
                );
            }
        }
        for warning in validation_warnings {
            if !self.warnings.contains(&warning) {
                self.warnings.push(warning);
            }
        }

        let timestamp = commit_timestamp(&self.durable_providers);
        let mut durable_at: Option<u64> = None;
        for durable in &self.durable_providers {
            let seq = durable
                .write_commit(principal.as_deref(), &self.changes, timestamp)
                .map_err(|error| GraphError::Durable {
                    reason: format!("{}: {error}", durable.provider_tag()),
                })?;
            durable_at = Some(durable_at.map_or(seq, |highest| highest.max(seq)));
        }

        self.pre_txn = None;
        self.snapshot.store(Arc::clone(&*self.guard));
        // Publish the schema-version bump AFTER snapshot.store so any reader
        // observing the new epoch is guaranteed to also observe the new
        // snapshot. Reverse ordering would let a reader read `epoch=N` and
        // then load the prior snapshot, planning against stale schema and
        // caching the plan under the new epoch (Codex PR #127 auto-review P1).
        if schema_changed {
            self.schema_version.fetch_add(1, Ordering::AcqRel);
        }

        let changes = std::mem::take(&mut self.changes);
        let warnings = std::mem::take(&mut self.warnings);

        // Hold guard + allocator across fanout. The thread-local fanout
        // guard increments a counter that `SharedGraph::begin_write` checks
        // before attempting any locking, so same-thread re-entrant writes
        // from inside `on_change` panic before reaching this lock — no
        // deadlock, and commit serialization is preserved. Concurrent
        // writers from other threads queue normally on the write lock.
        {
            let _fanout_guard = crate::reentry::FanoutGuard::enter();
            notify_providers(&self.providers, &changes);
        }

        metrics::counter_inc(metrics::COMMITS_TOTAL);
        metrics::histogram_record(
            metrics::COMMIT_DURATION_SECONDS,
            started.elapsed().as_secs_f64(),
        );
        metrics::gauge_set(metrics::GRAPH_NODES, self.read().node_count() as f64);
        metrics::gauge_set(metrics::GRAPH_EDGES, self.read().edge_count() as f64);

        Ok(CommitOutcome {
            generation,
            changes,
            principal,
            durable_at,
            next_node_id,
            next_edge_id,
            warnings,
        })
    }

    /// Roll back graph changes via `Drop` and release the write lock.
    pub fn rollback(self) {}

    /// Number of changes accumulated since this transaction opened.
    #[must_use]
    pub fn change_count(&self) -> usize {
        self.changes.len()
    }

    /// Whether this transaction has accumulated schema-changing work.
    #[must_use]
    pub fn has_schema_changes(&self) -> bool {
        self.changes
            .iter()
            .any(|change| matches!(change, Change::SchemaChanged { .. }))
    }
}

impl Drop for WriteTxn<'_> {
    fn drop(&mut self) {
        if let Some(prior) = self.pre_txn.take() {
            *self.guard = prior;
        }
    }
}

fn commit_timestamp(durable_providers: &[Arc<dyn DurableProvider>]) -> HlcTimestamp {
    durable_providers
        .first()
        .map_or_else(HlcTimestamp::zero, |provider| provider.next_timestamp())
}

/// Fan out committed changes to every registered provider, swallowing
/// returned errors and panics so a misbehaving provider can never abort or
/// crash the writer thread after the snapshot has already published.
///
/// Each iteration first reads the provider's tag through its own unwind
/// boundary so a panic in `provider_tag()` is logged with the sentinel
/// `<unknown>` tag and the provider is **skipped for this change** —
/// matching the original "single combined unwind" behavior where a tag
/// panic short-circuited `on_change`. When the tag is read successfully it
/// is reused in both the error-return and panic branches of `on_change` so
/// operators can attribute failures to the faulty provider.
#[tracing::instrument(
    name = "selene.graph.notify_providers",
    skip(providers, changes),
    fields(provider_count = providers.len(), change_count = changes.len())
)]
fn notify_providers(providers: &[Arc<dyn IndexProvider>], changes: &[Change]) {
    for provider in providers {
        for change in changes {
            // First boundary: cache the provider tag for logging. If
            // `provider_tag()` itself panics, log with the sentinel tag and
            // skip `on_change` — the provider is in an inconsistent state
            // and we should not invoke further side effects on it.
            let tag = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                provider.provider_tag()
            })) {
                Ok(tag) => tag,
                Err(payload) => {
                    let payload = describe_panic_payload(&payload);
                    tracing::error!(
                        provider_tag = %SENTINEL_PROVIDER_TAG,
                        ?change,
                        payload = %payload,
                        "index provider provider_tag() panicked after graph commit; \
                         skipping on_change for this change",
                    );
                    continue;
                }
            };

            // Second boundary: invoke `on_change`. The cached tag is
            // available for both the error-return and panic branches.
            // AssertUnwindSafe: provider interior state may be left
            // half-updated by a panic. The engine's contract is that the
            // graph commit succeeded; provider drift is logged but not
            // catastrophic.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                provider.on_change(change)
            }));
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::error!(
                        provider_tag = %tag,
                        error = %error,
                        ?change,
                        "index provider on_change failed after graph commit; continuing",
                    );
                }
                Err(panic_payload) => {
                    let payload = describe_panic_payload(&panic_payload);
                    tracing::error!(
                        provider_tag = %tag,
                        ?change,
                        payload = %payload,
                        "index provider on_change panicked after graph commit; continuing",
                    );
                }
            }
        }
    }
}

/// Sentinel value emitted on the `provider_tag` field when the provider's
/// own `provider_tag()` method panicked, so log filters keyed on the field
/// name still match.
const SENTINEL_PROVIDER_TAG: &str = "<unknown>";

fn describe_panic_payload(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_owned()
    }
}

#[cfg(test)]
mod tests;
