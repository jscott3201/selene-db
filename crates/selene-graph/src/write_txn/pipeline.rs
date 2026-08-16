use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::time::Instant;

use arc_swap::ArcSwap;
use selene_core::{Change, HlcTimestamp, metrics};

use crate::durable_provider::DurableProvider;
use crate::error::{GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::index_provider::IndexProvider;
use crate::write_txn::{CommitOutcome, CommitWarning, SealedCommit};

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
/// write lock in [`crate::write_txn::WriteTxn::seal`]; the committer never
/// re-validates, re-allocates ids, or re-applies a change list.
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
/// engine, Errs this commit + every already-appended batch member, and drains
/// the buffer.
///
/// Those members' bytes are **not** discarded by a reopen. This comment used to
/// claim they were "correct to lose on reopen" because the WAL never fsynced
/// them, and that is true only of losing the page cache — a machine crash. A
/// reopen is not a crash: under
/// [`SyncPolicy::OnFlushOnly`](selene_persist::SyncPolicy) `append_record`
/// hands a complete, checksum-valid frame to the kernel, recovery's tail repair
/// truncates only a torn tail, and such a frame is not torn. So the members
/// replay. That is why the committer reports
/// [`GraphError::IndeterminateOutcome`] rather than an unqualified rollback; see
/// that variant for the ISO §8.4 GR 1)b) reading. Making the old claim true is
/// a WAL-side change (a flushed-offset watermark truncated to on the poison
/// exit), tracked separately.
///
/// # Errors
///
/// Returns [`GraphError::Durable`] if a durable provider's `write_commit`
/// failed. The committer reclassifies it before it reaches a waiter.
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
            .write_commit(principal.as_ref(), &changes, timestamp)
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
        crate::provider_fanout::notify_providers(providers, generation, fanout);
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

/// Build a fan-out-only change list that substitutes each declarative truncate
/// change with the per-row tombstones the mutator staged for it.
///
/// Returns `None` when no truncate expansions are staged, so the common
/// (non-truncate) commit path fans out the persisted `changes` slice directly
/// with zero allocation. When expansions are present, every truncate change at
/// a staged index is replaced by its expansion (in order), and a truncate
/// change with no staged expansion (an empty-label no-op) is simply dropped
/// from fan-out — index providers see no tombstones because no rows were removed.
pub(super) fn expand_truncates_for_fanout(
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
            // live index providers reclaim derived state for every wiped
            // node/edge without seeing the bare declarative reset on the commit
            // path. WAL recovery still replays the persisted declarative change.
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

/// Test-only seam to drive a panic from inside [`publish_appended`] (Stage 3) on
/// a chosen publish ordinal, so the committer's multi-member publish-panic
/// poison-and-drain branch can be exercised deterministically.
///
/// This is the *only* way to reach that branch from a test: a misbehaving
/// [`IndexProvider`]'s `on_change` panic is swallowed by
/// [`crate::provider_fanout::notify_providers`]'s per-callback `catch_unwind`,
/// and `snapshot.store` does not panic, so without this hook the Stage-3 panic
/// path is unreachable through the public API. The counter is keyed on the
/// committer thread (the sole `publish_appended` caller), so "panic on the Nth
/// publish of this run" is deterministic. Compiled only under `cfg(test)`;
/// production builds have no injection point and no overhead.
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
