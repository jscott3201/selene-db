//! Write transaction RAII handle per spec 03 sections 4 and 6.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Instant;

use arc_swap::ArcSwap;
use parking_lot::{MutexGuard, RwLockWriteGuard};
use selene_core::{Change, HlcTimestamp, Origin, metrics};

use crate::committer::Committer;
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

/// A frozen, owned, `Send + 'static` commit bundle handed from a session thread
/// to the single committer thread (v1.2 multi-writer, BRIEF 1).
///
/// Produced by [`WriteTxn::seal`] **after** the generation/meta bump + GG02
/// validation have run under the write lock on the session thread (so error
/// timing is unchanged), and after the lock + allocator guards have been
/// released. It contains the fully-built next snapshot plus everything the
/// committer needs to run the durable+publish tail — **no guards, no graph
/// reference, no borrow**. The committer never re-validates, re-allocates ids,
/// or re-applies a change list; it just stamps the HLC, appends to the WAL,
/// publishes the frozen snapshot, and bumps the schema epoch.
///
/// The HLC timestamp is deliberately **not** stamped here: the committer stamps
/// it per bundle in FIFO drain order so HLC is monotonic in commit order
/// (== publish order == seal order). Stamping it on the session thread would
/// break that monotonicity once seal-order and stamp-order could diverge.
pub struct SealedCommit {
    /// Fully-built next snapshot, frozen under the session's write lock.
    pub(crate) next_snapshot: Arc<SeleneGraph>,
    /// Persisted change list (the WAL/changeset payload).
    pub(crate) changes: Vec<Change>,
    /// Truncate-expanded fan-out view, built on the session thread, or `None`
    /// when no truncate/reset expansion is staged (the common path).
    pub(crate) fanout_changes: Option<Vec<Change>>,
    /// Opaque caller-supplied principal bytes for the WAL entry header (D12).
    pub(crate) principal: Option<Arc<[u8]>>,
    /// Whether the change list bumps the schema epoch.
    pub(crate) schema_changed: bool,
    /// Already-bumped graph generation.
    pub(crate) generation: u64,
    /// Next node id after this commit (peeked under the lock).
    pub(crate) next_node_id: u64,
    /// Next edge id after this commit (peeked under the lock).
    pub(crate) next_edge_id: u64,
    /// Non-fatal validation warnings collected during seal.
    pub(crate) warnings: Vec<CommitWarning>,
    /// BRIEF-117 cancellation cut-line token. Checked once by the committer
    /// immediately **before** `write_commit`: a pre-WAL cancel returns
    /// `Cancelled` and appends nothing (no id burned beyond the monotonic hole
    /// already allocated under the lock); past the append it is irrevocable.
    pub(crate) cancel: Option<Arc<AtomicBool>>,
}

impl SealedCommit {
    /// Attach a BRIEF-117 cancellation token so the committer can abort this
    /// commit at the pre-WAL cut-line.
    ///
    /// The token is sampled exactly once, immediately before the WAL append.
    /// Once the append has run the commit is irrevocable. With no token (the
    /// default produced by [`WriteTxn::seal`]) the commit is never cancelled.
    #[must_use]
    pub fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }
}

/// Compile-time proof that [`SealedCommit`] is `Send + 'static`, so it can be
/// moved to the committer thread. A guard or borrow leaking into the struct
/// would fail this assertion at compile time.
const _: fn() = || {
    fn assert_send_static<T: Send + 'static>() {}
    assert_send_static::<SealedCommit>();
};

/// RAII owner of the single graph write lock.
///
/// Since v1.2 (BRIEF 1) the transaction no longer holds the snapshot cell,
/// schema-version, or provider handles — those moved to the single committer
/// thread, which the transaction reaches via the cheap [`Committer`] submit
/// handle. The transaction still owns the write lock + allocator guards for the
/// duration of execution and releases them when [`Self::seal`] consumes it.
pub struct WriteTxn<'g> {
    pub(crate) guard: RwLockWriteGuard<'g, Arc<SeleneGraph>>,
    pub(crate) committer: Committer,
    pub(crate) pre_txn: Option<Arc<SeleneGraph>>,
    pub(crate) allocator: MutexGuard<'g, IdAllocator>,
    /// Index-provider registry, retained so `Mutator::index_provider_by_tag`
    /// can resolve a provider during execution. The committer holds its own
    /// clone for fan-out; this is the execution-time lookup copy.
    pub(crate) providers: Vec<Arc<dyn IndexProvider>>,
    pub(crate) changes: Vec<Change>,
    /// Per-truncate per-row tombstone expansions, keyed by the index of the
    /// declarative truncate change in [`Self::changes`] that produced them.
    ///
    /// BRIEF-150 / deletion-reclamation audit Item 11. The WAL/changeset carries
    /// only the O(1) declarative `NodesOfTypeTruncated`/`EdgesOfTypeTruncated`
    /// change, but index-provider fan-out must observe the same per-row
    /// `NodeDeleted`/`EdgeDeleted` multiset a `MATCH (n:L) DETACH DELETE n` would
    /// emit (so derived state is reclaimed without leaks). The mutator
    /// snapshots the matched ids while it still holds the store and stages their
    /// tombstones here; commit substitutes each truncate change with its staged
    /// expansion before fan-out.
    pub(crate) truncate_expansions: Vec<(usize, Vec<Change>)>,
    pub(crate) warnings: Vec<CommitWarning>,
}

impl<'g> WriteTxn<'g> {
    pub(crate) fn new(
        guard: RwLockWriteGuard<'g, Arc<SeleneGraph>>,
        committer: Committer,
        allocator: MutexGuard<'g, IdAllocator>,
        providers: Vec<Arc<dyn IndexProvider>>,
    ) -> Self {
        let pre_txn = Some(Arc::clone(&*guard));
        Self {
            guard,
            committer,
            pre_txn,
            allocator,
            providers,
            changes: Vec::new(),
            truncate_expansions: Vec::new(),
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
    /// Since v1.2 (BRIEF 1) commit is **seal-and-handover**: this method runs
    /// [`Self::seal`] on the calling thread (generation/meta bump + GG02
    /// validation under the write lock, then **lock release**), then submits the
    /// resulting [`SealedCommit`] to the per-graph single committer thread and
    /// blocks until it is durable + visible. The public contract is unchanged —
    /// "`commit()` returns ⇒ durable + visible" — only the internal threading
    /// model differs.
    ///
    /// GG02 closed-graph violations still abort here, on the calling thread,
    /// before any handoff, so error timing is identical to v1.0/v1.1.
    ///
    /// Registered index providers are notified by the committer after the new
    /// snapshot is published; the same-thread re-entrancy guard now protects the
    /// committer thread (one committer ⇒ still sound). Same-thread re-entrant
    /// provider calls into `SharedGraph::begin_write()` are detected via the
    /// thread-local fanout counter and panic with a clear message; the
    /// committer's fan-out boundary catches those panics (along with
    /// callback-internal panics and returned errors) so a single misbehaving
    /// provider can never crash the committer thread.
    ///
    /// # Errors
    ///
    /// Returns the GG02 / validation error from [`Self::seal`], or a
    /// [`GraphError::Durable`] if the WAL append failed or the committer thread
    /// is no longer running.
    #[tracing::instrument(
        name = "selene.graph.commit",
        skip(self, principal),
        fields(change_count = self.change_count())
    )]
    pub fn commit_with_principal(self, principal: Option<Arc<[u8]>>) -> GraphResult<CommitOutcome> {
        // Clone the submit handle BEFORE sealing — `seal()` consumes `self`
        // (dropping the write lock + allocator guards as it returns), so the
        // session releases the lock strictly before it enqueues and blocks on
        // `recv()`. This ordering is the load-bearing deadlock invariant: a
        // session is never simultaneously lock-holding and recv-blocked, so it
        // cannot wedge a queued `Compact` for which the committer must take the
        // same lock (v1.2 design §3.2).
        let committer = self.committer.clone();
        let sealed = self.seal(principal)?;
        committer.submit_commit(sealed)
    }

    /// Run the under-lock half of commit and hand back an owned, `Send`
    /// [`SealedCommit`] for the committer thread (v1.2 multi-writer, BRIEF 1).
    ///
    /// Runs, on the calling thread under the held write lock: schema-change
    /// detection, the generation/meta bump (`generation += 1` + write the next
    /// ids into `GraphMeta`), and GG02 closed-graph validation — which **still
    /// aborts synchronously here** (the `?` propagates and `Drop` rolls back the
    /// generation bump, exactly as in v1.0/v1.1). Then it disarms `Drop`
    /// (`pre_txn = None`), clones the now-frozen next snapshot, `mem::take`s the
    /// change list / truncate expansions / warnings, builds the truncate-expanded
    /// fan-out view, and returns. The write lock + allocator guards drop as this
    /// method returns (it consumes `self`).
    ///
    /// The HLC timestamp is **not** stamped here (see [`SealedCommit`]); the
    /// committer stamps it per bundle in FIFO drain order.
    ///
    /// # Errors
    ///
    /// Returns the GG02 / closed-graph validation error
    /// ([`GraphError::TypeViolation`]) when a change violates the bound type.
    pub fn seal(mut self, principal: Option<Arc<[u8]>>) -> GraphResult<SealedCommit> {
        debug_assert!(
            self.pre_txn.is_some(),
            "pre_txn must be present at seal entry"
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
                    // NB: `?` here returns Err with the generation bump still
                    // applied to the guard-Arc; `Drop` then restores `pre_txn`
                    // and undoes the bump — error timing + rollback unchanged.
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

        // Disarm Drop-rollback: from here the commit is handed to the committer
        // and the in-place mutations become the published state.
        self.pre_txn = None;
        // Freeze the next snapshot under the lock. The committer publishes this
        // exact Arc and never rebuilds it.
        let next_snapshot = Arc::clone(&*self.guard);

        let changes = std::mem::take(&mut self.changes);
        let truncate_expansions = std::mem::take(&mut self.truncate_expansions);
        let warnings = std::mem::take(&mut self.warnings);

        // BRIEF-150 / audit Item 11: build the fan-out-only truncate-expanded
        // view on the session thread so the committer holds a fully-owned
        // bundle. `None` on the common (non-truncate) path → zero allocation.
        let fanout_changes = expand_truncates_for_fanout(&changes, &truncate_expansions);

        Ok(SealedCommit {
            next_snapshot,
            changes,
            fanout_changes,
            principal,
            schema_changed,
            generation,
            next_node_id,
            next_edge_id,
            warnings,
            cancel: None,
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

/// Run the durable + publish tail of a sealed commit on the committer thread
/// (v1.2 multi-writer, BRIEF 1). This is the second half of what
/// `commit_with_principal` did under the lock pre-v1.2, in the **identical**
/// order — only the thread changed.
///
/// Ordering (verbatim from the pre-v1.2 commit body):
/// 1. Stamp the HLC timestamp **here**, in committer FIFO order, so HLC is
///    monotonic in commit order (== publish order).
/// 2. BRIEF-117 cut-line: sample `cancel` once; if set, return `Cancelled`
///    before any `write_commit` — nothing is appended or published.
/// 3. WAL-first: `write_commit` for each durable provider; its error aborts the
///    commit (and BRIEF 1 keeps `EveryN(1)`, so this also fsyncs). Past this
///    point the commit is irrevocable.
/// 4. Publish: `snapshot.store(next_snapshot)` (the ArcSwap linearization point;
///    the committer is its sole writer).
/// 5. store-before-schema-bump (PR #127 P1): bump `schema_version` strictly
///    after the store.
/// 6. Debug-only index-consistency assertion on the just-stored snapshot.
/// 7. No-op provider fan-out under the [`FanoutGuard`] (now committer-thread-
///    local — sound with one committer).
///
/// # Errors
///
/// Returns [`GraphError::Cancelled`] at the pre-WAL cut-line, or
/// [`GraphError::Durable`] if a durable provider's `write_commit` failed.
pub(crate) fn publish_sealed(
    sealed: SealedCommit,
    snapshot: &ArcSwap<SeleneGraph>,
    schema_version: &AtomicU64,
    providers: &[Arc<dyn IndexProvider>],
    durable_providers: &[Arc<dyn DurableProvider>],
) -> GraphResult<CommitOutcome> {
    let started = Instant::now();
    let SealedCommit {
        next_snapshot,
        changes,
        fanout_changes,
        principal,
        schema_changed,
        generation,
        next_node_id,
        next_edge_id,
        warnings,
        cancel,
    } = sealed;

    // (1) Stamp the HLC in committer FIFO order (monotonic in commit order).
    let timestamp = commit_timestamp(durable_providers);

    // (2) BRIEF-117 pre-WAL cut-line: sample the token exactly once. A cancel
    // observed here appends nothing and publishes nothing; the monotonic id
    // hole already allocated under the session's lock simply stays a permanent
    // hole (D11/D22), exactly as an aborted transaction would leave it.
    if let Some(flag) = &cancel
        && flag.load(Ordering::Acquire)
    {
        return Err(GraphError::Cancelled);
    }

    // (3) WAL-first: the append gates the commit. Irrevocable past here.
    let mut durable_at: Option<u64> = None;
    for durable in durable_providers {
        let seq = durable
            .write_commit(principal.as_deref(), &changes, timestamp)
            .map_err(|error| GraphError::Durable {
                reason: format!("{}: {error}", durable.provider_tag()),
            })?;
        durable_at = Some(durable_at.map_or(seq, |highest| highest.max(seq)));
    }

    // (4) Publish — the sole ArcSwap writer.
    snapshot.store(Arc::clone(&next_snapshot));
    // (5) Publish the schema-version bump AFTER snapshot.store so any reader
    // observing the new epoch is guaranteed to also observe the new snapshot
    // (Codex PR #127 auto-review P1). Reverse ordering would let a reader read
    // `epoch=N` then load the prior snapshot, planning against stale schema.
    if schema_changed {
        schema_version.fetch_add(1, Ordering::AcqRel);
    }

    // (6) Debug-only structural net on the exact snapshot just stored (a pure
    // read of the immutable snapshot — never re-enters begin_write). Now runs
    // on the committer thread; compiled out in release builds.
    #[cfg(debug_assertions)]
    if let Err(reason) = next_snapshot.assert_indexes_consistent() {
        panic!("selene-graph: post-commit index consistency violation: {reason}");
    }

    // (7) No-op provider fan-out. The FanoutGuard's thread-local counter now
    // guards the COMMITTER thread: a provider that re-enters begin_write on the
    // committer thread panics before locking, and the boundary below catches
    // it. Safe with exactly one committer (v1.2 design §7.7).
    let fanout: &[Change] = fanout_changes.as_deref().unwrap_or(&changes);
    {
        let _fanout_guard = crate::reentry::FanoutGuard::enter();
        notify_providers(providers, fanout);
    }

    metrics::counter_inc(metrics::COMMITS_TOTAL);
    metrics::histogram_record(
        metrics::COMMIT_DURATION_SECONDS,
        started.elapsed().as_secs_f64(),
    );
    metrics::gauge_set(metrics::GRAPH_NODES, next_snapshot.node_count() as f64);
    metrics::gauge_set(metrics::GRAPH_EDGES, next_snapshot.edge_count() as f64);

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
                    let payload = crate::panic_payload::describe(&payload);
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
                    let payload = crate::panic_payload::describe(&panic_payload);
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

/// Build a fan-out-only change list that substitutes each declarative truncate
/// change with the per-row tombstones the mutator staged for it.
///
/// Returns `None` when no truncate expansions are staged, so the common
/// (non-truncate) commit path fans out the persisted `changes` slice directly
/// with zero allocation. When expansions are present, every truncate change at
/// a staged index is replaced by its expansion (in order), and a truncate
/// change with no staged expansion (an empty-label no-op) is simply dropped
/// from fan-out — index providers see no tombstones because no rows were removed.
fn expand_truncates_for_fanout(
    changes: &[Change],
    expansions: &[(usize, Vec<Change>)],
) -> Option<Vec<Change>> {
    if expansions.is_empty() {
        return None;
    }
    let mut view = Vec::with_capacity(changes.len());
    for (index, change) in changes.iter().enumerate() {
        match change {
            // BRIEF-152: GraphReset is fanned out as its staged per-row
            // tombstones too, alongside the BRIEF-150 truncate variants, so
            // index providers reclaim derived state for every wiped node/edge and
            // never see the bare declarative reset they could not expand.
            Change::NodesOfTypeTruncated { .. }
            | Change::EdgesOfTypeTruncated { .. }
            | Change::GraphReset { .. } => {
                if let Some((_, expansion)) = expansions.iter().find(|(staged, _)| *staged == index)
                {
                    view.extend(expansion.iter().cloned());
                }
                // A truncate/reset change with no staged expansion removed zero
                // rows (empty/absent label, or a reset of an empty graph); it
                // contributes nothing to fan-out.
            }
            other => view.push(other.clone()),
        }
    }
    Some(view)
}

/// Sentinel value emitted on the `provider_tag` field when the provider's
/// own `provider_tag()` method panicked, so log filters keyed on the field
/// name still match.
const SENTINEL_PROVIDER_TAG: &str = "<unknown>";

#[cfg(test)]
mod tests;
