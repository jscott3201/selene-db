//! Write transaction RAII handle per spec 03 sections 4 and 6.

use std::sync::mpsc::SyncSender;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Instant;

use arc_swap::ArcSwap;
use parking_lot::{MutexGuard, RwLockWriteGuard};
use selene_core::{Change, HlcTimestamp, metrics};

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
/// it per bundle in **seal-sequence** drain order so HLC is monotonic in commit
/// order (== publish order == seal order). Stamping it on the session thread
/// would break that monotonicity once seal-order and stamp-order diverge.
///
/// # Why a `seal_seq` (publish-order correctness, P0 fix)
///
/// `seal()` consumes the [`WriteTxn`], so the write lock + allocator guards drop
/// as it returns — **before** the caller enqueues the bundle. Two sessions can
/// therefore seal in lock order (A then B) yet `send()` in the opposite order
/// (B before A) if A is preempted between lock-release and send. A naive FIFO
/// committer would then publish B's gen-`N+1` snapshot before A's gen-`N`
/// snapshot, regressing the published snapshot and losing A's older view under
/// B — a D10 serializability violation. To prevent it, `seal()` stamps a
/// strictly-monotonic `seal_seq` **while still holding the write lock** (so
/// seal-seq order == lock-acquisition order == the intended total order), and
/// the committer publishes strictly in `seal_seq` order via a reorder buffer,
/// regardless of channel arrival order. Compaction takes a `seal_seq` from the
/// same counter under the same lock, so a compact can never be reordered ahead
/// of an earlier-sealed commit.
pub(crate) struct SealedCommit {
    /// Strictly-monotonic publish-order key, allocated under the write lock in
    /// [`WriteTxn::seal`]. The committer publishes in ascending `seal_seq`.
    pub(crate) seal_seq: u64,
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
        Mutator::new(self)
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
        // `recv()`. The committer never takes the write lock (compaction builds
        // on the caller thread too, since v1.2 BRIEF 1 P0 fix), so a session is
        // never simultaneously lock-holding and recv-blocked.
        let committer = self.committer.clone();
        let sealed = self.seal(principal, None)?;
        committer.submit_commit(sealed)
    }

    /// Run the under-lock half of commit and hand back an owned, `Send`
    /// [`SealedCommit`] for the committer thread (v1.2 multi-writer, BRIEF 1).
    ///
    /// Runs, on the calling thread under the held write lock: schema-change
    /// detection, the generation/meta bump (`generation += 1` + write the next
    /// ids into `GraphMeta`), and GG02 closed-graph validation — which **still
    /// aborts synchronously here** (the `?` propagates and `Drop` rolls back the
    /// generation bump, exactly as in v1.0/v1.1). It then samples the optional
    /// BRIEF-117 cancellation token **while the lock is still held and before
    /// `Drop` is disarmed** (see below), allocates the strictly-monotonic
    /// `seal_seq` under the lock, disarms `Drop` (`pre_txn = None`), clones the
    /// now-frozen next snapshot, `mem::take`s the change list / truncate
    /// expansions / warnings, builds the truncate-expanded fan-out view, and
    /// returns. The write lock + allocator guards drop as this method returns
    /// (it consumes `self`).
    ///
    /// # Cancellation cut-line (BRIEF-117, P0 fix)
    ///
    /// The cancellation token is sampled **here, under the write lock, before
    /// disarming `Drop`** — not on the committer before the WAL append. In the
    /// seal-and-handover model multiple commits can be sealed-but-unpublished at
    /// once, each forking `*shared` off the previous; a commit's mutation is
    /// therefore already woven into `*shared` (and possibly built upon by a
    /// later seal) by the time the committer would run a pre-WAL check, so a
    /// committer-side cancel could not surgically remove it without poisoning
    /// the engine. Sampling under the lock means a cancelled commit is rolled
    /// back by `Drop` exactly like a GG02 abort — `*shared` is restored, no
    /// `seal_seq` is consumed, nothing is enqueued — so the cut-line's
    /// guarantee ("no append, no publish, exactly as an aborted transaction
    /// would leave it") is *literally* true. A cancel observed after `seal`
    /// returns is too late: the commit is already in flight and irrevocable.
    ///
    /// The HLC timestamp is **not** stamped here (see [`SealedCommit`]); the
    /// committer stamps it per bundle in seal-sequence drain order.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Cancelled`] when `cancel` is set at entry (rolled
    /// back via `Drop`), or the GG02 / closed-graph validation error
    /// ([`GraphError::TypeViolation`]) when a change violates the bound type.
    pub(crate) fn seal(
        mut self,
        principal: Option<Arc<[u8]>>,
        cancel: Option<&AtomicBool>,
    ) -> GraphResult<SealedCommit> {
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

        // BRIEF-117 cut-line: sample the cancellation token while the lock is
        // still held and `Drop` is still armed. A cancel here returns Err with
        // the generation bump + staged mutations still on the guard-Arc; `Drop`
        // then restores `pre_txn`, rolling everything back exactly as a GG02
        // abort or an aborted transaction would. Nothing is enqueued or
        // published, and no `seal_seq` is consumed.
        if let Some(flag) = cancel
            && flag.load(Ordering::Acquire)
        {
            return Err(GraphError::Cancelled);
        }

        // Allocate the publish-order key under the lock so seal-seq order equals
        // lock-acquisition order (the intended total order). Done after every
        // fallible step so an aborted seal consumes no sequence number, keeping
        // the committer's reorder sequence gap-free.
        let seal_seq = self.committer.next_seal_seq();

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
            seal_seq,
            next_snapshot,
            changes,
            fanout_changes,
            principal,
            schema_changed,
            generation,
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

/// A sealed commit whose durable bytes have been **appended** to every durable
/// provider but **not yet fsynced or published** (v1.2 multi-writer, BRIEF 2).
///
/// This is the intermediate state between Stage 1 ([`append_sealed`]) and
/// Stage 3 ([`publish_appended`]) of the group-commit pipeline. The committer
/// forms a contiguous run of `AppendedCommit`s, runs ONE group flush
/// ([`flush_durables`] — the R1 fsync-before-publish barrier) over the whole
/// run, then publishes + acks each in `seal_seq` order. Because the flush is the
/// single barrier for the whole batch, an appended-but-unflushed commit is
/// **never** published or acked — it is only ever lost atomically with the rest
/// of its run on a crash (durable-before-visible).
///
/// It owns the reply [`SyncSender`] (moved in by the committer when it pops the
/// `Work::Commit`) so the Stage-4 ack is a single drain in `seal_seq` order with
/// no parallel-`Vec` correlation between commits and their reply channels.
///
/// All post-append/pre-publish state carried here was frozen under the session's
/// write lock in [`WriteTxn::seal`]; the committer never re-validates,
/// re-allocates ids, or re-applies a change list.
pub(crate) struct AppendedCommit {
    /// Fully-built next snapshot, frozen under the session's write lock. The
    /// committer stores this exact `Arc` in Stage 3 and never rebuilds it.
    pub(crate) next_snapshot: Arc<SeleneGraph>,
    /// Persisted change list, returned in the [`CommitOutcome`].
    pub(crate) changes: Vec<Change>,
    /// Truncate-expanded fan-out view, or `None` on the common (non-truncate)
    /// path (then fan-out uses `changes` directly).
    pub(crate) fanout_changes: Option<Vec<Change>>,
    /// Opaque caller-supplied principal bytes (D12), returned in the outcome.
    pub(crate) principal: Option<Arc<[u8]>>,
    /// Whether the change list bumps the schema epoch (store-before-schema-bump).
    pub(crate) schema_changed: bool,
    /// Already-bumped graph generation.
    pub(crate) generation: u64,
    /// Next node id after this commit.
    pub(crate) next_node_id: u64,
    /// Next edge id after this commit.
    pub(crate) next_edge_id: u64,
    /// Non-fatal validation warnings.
    pub(crate) warnings: Vec<CommitWarning>,
    /// Highest durable sequence assigned across the durable providers during
    /// [`append_sealed`]. Observable only after the group flush + publish.
    pub(crate) durable_at: Option<u64>,
    /// The reply channel for this commit's waiter, set by the committer when it
    /// pops the `Work::Commit`. Drained (acked) in Stage 4, in `seal_seq` order.
    pub(crate) reply: Option<SyncSender<GraphResult<CommitOutcome>>>,
    /// `Instant` captured at append time so commit-duration metrics span the
    /// full durable+publish tail (recorded in [`publish_appended`]).
    pub(crate) started: Instant,
}

/// Compile-time proof that [`AppendedCommit`] is `Send + 'static`, so the
/// committer can hold a batch of them across the group flush.
const _: fn() = || {
    fn assert_send_static<T: Send + 'static>() {}
    assert_send_static::<AppendedCommit>();
};

/// Stage 1 — **append** a sealed commit's durable bytes to every durable
/// provider, with fsync **deferred** (v1.2 multi-writer, BRIEF 2). Performs no
/// store, no schema bump, and no fan-out: the snapshot is not yet visible.
///
/// This is the first third of the pre-BRIEF-2 `publish_sealed` body, split at
/// the append/store seam so the committer can append a whole contiguous run
/// before fsyncing it once (group commit). Ordering within Stage 1 is verbatim:
/// 1. Debug-only index-consistency assertion on the frozen `next_snapshot`,
///    BEFORE any durable append. A detected violation aborts with nothing
///    durable and nothing visible (it poisons via the committer's
///    `catch_unwind`, but no WAL entry was written and the published cell never
///    advanced, so a reopen is clean). Asserting after the store (pre-fix) would
///    return `Err` for a commit that *did* persist — a P2 inversion.
/// 2. Stamp the HLC in committer seal-sequence order so HLC is monotonic in
///    commit order (== publish order).
/// 3. WAL-first: `write_commit` for each durable provider. Under the BRIEF-2
///    `OnFlushOnly` policy this append does **not** fsync — the committer's
///    later [`flush_durables`] is the single fsync for the whole run. The
///    returned per-provider sequence is folded into `durable_at`.
///
/// The split makes durability **fsync-gated, not append-gated**: an appended
/// commit is durable only after the group [`flush_durables`] returns `Ok`. The
/// committer holds the append's bytes-but-no-fsync state in the returned
/// [`AppendedCommit`] and publishes/acks strictly after the barrier.
///
/// # Failure ⇒ engine poison (handled by the committer)
///
/// A returned `Err` is a **post-seal** failure: `seal()` already wove this
/// commit's mutation into `*shared` (and a later seal may have forked off it),
/// so the live graph cannot be surgically rolled back. The committer poisons the
/// engine, Errs this commit + every already-appended batch member (whose
/// appended-but-unflushed bytes are correct to lose on reopen), and drains the
/// buffer. The durable WAL never fsynced any of them, so a reopen heals.
///
/// # Errors
///
/// Returns [`GraphError::Durable`] if a durable provider's `write_commit`
/// failed.
pub(crate) fn append_sealed(
    sealed: SealedCommit,
    durable_providers: &[Arc<dyn DurableProvider>],
) -> GraphResult<AppendedCommit> {
    let started = Instant::now();
    let SealedCommit {
        seal_seq: _,
        next_snapshot,
        changes,
        fanout_changes,
        principal,
        schema_changed,
        generation,
        next_node_id,
        next_edge_id,
        warnings,
    } = sealed;

    // (1) Debug-only structural net on the frozen snapshot, BEFORE any durable
    // append. The snapshot is immutable from seal time, so this is just as sound
    // as asserting after the store — but a detected violation now aborts with
    // nothing durable and nothing visible (no `Err`-but-committed inversion). A
    // pure read of the immutable snapshot — never re-enters begin_write.
    // Compiled out in release builds.
    #[cfg(debug_assertions)]
    if let Err(reason) = next_snapshot.assert_indexes_consistent() {
        panic!("selene-graph: pre-publish index consistency violation: {reason}");
    }

    // (2) Stamp the HLC in committer seal-sequence order (monotonic in commit
    // order).
    let timestamp = commit_timestamp(durable_providers);

    // (3) WAL-first append. Under OnFlushOnly (BRIEF 2) this does NOT fsync —
    // the committer's group flush is the single barrier. A returned error
    // poisons the committer (the session-thread seal already mutated `*shared`
    // and cannot be rolled back).
    let mut durable_at: Option<u64> = None;
    for durable in durable_providers {
        let seq = durable
            .write_commit(principal.as_deref(), &changes, timestamp)
            .map_err(|error| GraphError::Durable {
                reason: format!("{}: {error}", durable.provider_tag()),
            })?;
        durable_at = Some(durable_at.map_or(seq, |highest| highest.max(seq)));
    }

    Ok(AppendedCommit {
        next_snapshot,
        changes,
        fanout_changes,
        principal,
        schema_changed,
        generation,
        next_node_id,
        next_edge_id,
        warnings,
        durable_at,
        reply: None,
        started,
    })
}

/// Stage 2 — **flush** every durable provider, fsyncing the whole contiguous run
/// of appended commits in one barrier (the R1 fsync-before-publish barrier,
/// v1.2 multi-writer, BRIEF 2).
///
/// Called by the committer exactly once per drained run of [`append_sealed`]s,
/// strictly **before** any of the run's commits are published or acked. After it
/// returns `Ok`, every byte appended in the run is durable, so publishing the
/// snapshots (Stage 3) and delivering `durable_at` (Stage 4) cannot expose a
/// not-yet-durable commit (durable-before-visible). When `commit_batching=Off`
/// the run length is capped at 1, so this is exactly one fsync per commit at the
/// same order point as `EveryN(1)`'s append-time fsync — behaviorally identical
/// to BRIEF 1.
///
/// # Failure ⇒ engine poison (handled by the committer)
///
/// A flush error means the run's appended bytes may not be durable. The
/// committer publishes **nothing** from the run, poisons the engine, and Errs
/// every member — reopen lets WAL recovery decide truth, and no caller was told
/// "durable."
///
/// # Errors
///
/// Returns [`GraphError::Durable`] if a durable provider's `flush` failed.
pub(crate) fn flush_durables(durable_providers: &[Arc<dyn DurableProvider>]) -> GraphResult<()> {
    for durable in durable_providers {
        durable.flush().map_err(|error| GraphError::Durable {
            reason: format!("{}: {error}", durable.provider_tag()),
        })?;
    }
    Ok(())
}

/// Stage 3+4 — make an appended (and now group-flushed) commit **visible** and
/// build its [`CommitOutcome`] (v1.2 multi-writer, BRIEF 2). **Infallible.**
///
/// Called by the committer only after [`flush_durables`] returned `Ok` for the
/// whole run, in `seal_seq` order. Infallibility is load-bearing: by the time we
/// store the snapshot the commit is already durable, so there is no honest way
/// to return `Err` here — returning `Result` would reintroduce the P2
/// "returns-Err-but-actually-published" inversion. The committer still wraps the
/// call in `catch_unwind`, so a `store`/debug-assert PANIC still poisons.
///
/// Ordering (verbatim from the pre-BRIEF-2 `publish_sealed` tail):
/// 1. Publish: `snapshot.store(next_snapshot)` (the ArcSwap linearization point;
///    the committer is its sole writer).
/// 2. store-before-schema-bump (PR #127 P1): bump `schema_version` strictly
///    after the store, so a reader seeing the new epoch also sees the new
///    snapshot.
/// 3. No-op provider fan-out under the [`crate::reentry::FanoutGuard`] (now
///    committer-thread-local — sound with one committer).
/// 4. Metrics + build the [`CommitOutcome`] (carrying the `durable_at` computed
///    in [`append_sealed`]).
pub(crate) fn publish_appended(
    appended: AppendedCommit,
    snapshot: &ArcSwap<SeleneGraph>,
    schema_version: &AtomicU64,
    providers: &[Arc<dyn IndexProvider>],
) -> CommitOutcome {
    let AppendedCommit {
        next_snapshot,
        changes,
        fanout_changes,
        principal,
        schema_changed,
        generation,
        next_node_id,
        next_edge_id,
        warnings,
        durable_at,
        reply: _,
        started,
    } = appended;

    // Test-only injection seam for the Stage-3 publish-panic path (BRIEF 2 crash
    // matrix item 6). In production this compiles to nothing. It panics BEFORE the
    // store so the panicking member never publishes — matching the real
    // store/debug-assert panic this branch defends against — letting a test drive
    // the "member i of a multi-member batch panics ⇒ i acked-and-visible,
    // panicking i+1 + remaining i+2.. Err'd, poisoned, drained" path that
    // `notify_providers`' panic-swallowing makes otherwise unreachable.
    #[cfg(test)]
    publish_panic_inject::maybe_panic();

    // (1) Publish — the sole ArcSwap writer.
    snapshot.store(Arc::clone(&next_snapshot));
    // (2) Publish the schema-version bump AFTER snapshot.store so any reader
    // observing the new epoch is guaranteed to also observe the new snapshot
    // (Codex PR #127 auto-review P1). Reverse ordering would let a reader read
    // `epoch=N` then load the prior snapshot, planning against stale schema.
    if schema_changed {
        schema_version.fetch_add(1, Ordering::AcqRel);
    }

    // (3) No-op provider fan-out. The FanoutGuard's thread-local counter now
    // guards the COMMITTER thread: a provider that re-enters begin_write on the
    // committer thread panics before locking, and the boundary below catches
    // it. Safe with exactly one committer (v1.2 design §7.7).
    let fanout: &[Change] = fanout_changes.as_deref().unwrap_or(&changes);
    {
        let _fanout_guard = crate::reentry::FanoutGuard::enter();
        notify_providers(providers, fanout);
    }

    // (4) Metrics + outcome.
    metrics::counter_inc(metrics::COMMITS_TOTAL);
    metrics::histogram_record(
        metrics::COMMIT_DURATION_SECONDS,
        started.elapsed().as_secs_f64(),
    );
    metrics::gauge_set(metrics::GRAPH_NODES, next_snapshot.node_count() as f64);
    metrics::gauge_set(metrics::GRAPH_EDGES, next_snapshot.edge_count() as f64);

    CommitOutcome {
        generation,
        changes,
        principal,
        durable_at,
        next_node_id,
        next_edge_id,
        warnings,
    }
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

/// Test-only seam to drive a panic from inside [`publish_appended`] (Stage 3) on
/// a chosen publish ordinal, so the committer's multi-member publish-panic
/// poison-and-drain branch can be exercised deterministically.
///
/// This is the *only* way to reach that branch from a test: a misbehaving
/// [`IndexProvider`]'s `on_change` panic is swallowed by [`notify_providers`]'
/// per-callback `catch_unwind`, and `snapshot.store` does not panic, so without
/// this hook the Stage-3 panic path is unreachable through the public API. The
/// counter is keyed on the committer thread (the sole `publish_appended` caller),
/// so "panic on the Nth publish of this run" is deterministic. Compiled only
/// under `cfg(test)`; production builds have no injection point and no overhead.
#[cfg(test)]
pub(crate) mod publish_panic_inject {
    use std::cell::Cell;

    thread_local! {
        /// `Some(remaining)` arms the injection: each [`maybe_panic`] call
        /// decrements it, panicking when it reaches zero. `None` (the default) is
        /// disarmed. Thread-local so only the committer thread that ran
        /// [`arm`] is affected, and it auto-clears after firing.
        static COUNTDOWN: Cell<Option<u32>> = const { Cell::new(None) };
    }

    /// Arm the injection so the `after`-th subsequent [`maybe_panic`] panics
    /// (`after = 1` ⇒ the next publish panics; `after = 2` ⇒ the second). Must be
    /// called on the committer thread (e.g. from inside a provider `on_change`
    /// during an earlier publish in the same run).
    pub(crate) fn arm(after: u32) {
        COUNTDOWN.with(|cell| cell.set(Some(after)));
    }

    /// Panic if armed and the countdown has elapsed; otherwise a no-op. Clears the
    /// arming when it fires so exactly one panic is injected per [`arm`].
    pub(crate) fn maybe_panic() {
        COUNTDOWN.with(|cell| {
            if let Some(remaining) = cell.get() {
                let next = remaining.saturating_sub(1);
                if next == 0 {
                    cell.set(None);
                    panic!("selene-graph test: injected Stage-3 publish_appended panic");
                }
                cell.set(Some(next));
            }
        });
    }
}

#[cfg(test)]
mod tests;
