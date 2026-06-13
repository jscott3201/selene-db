//! Single per-graph commit thread — the sole publisher of the live snapshot.
//!
//! # Why a dedicated committer (v1.2 multi-writer, BRIEF 1)
//!
//! Before v1.2 every writer thread published its own snapshot under the held
//! write lock (`WriteTxn::commit_with_principal`). v1.2 splits commit into two
//! halves (see [`crate::write_txn::WriteTxn::seal`]):
//!
//! 1. **seal** (session thread, under the lock): generation/meta bump + GG02
//!    validation + build the frozen next snapshot, then **release the lock**.
//! 2. **publish tail** (this committer thread, FIFO): HLC stamp → WAL append →
//!    `snapshot.store` → store-before-schema-bump → no-op provider fan-out.
//!
//! The committer is the **sole writer of the [`ArcSwap`] snapshot cell**. That
//! single-writer + seal-sequence-ordered discipline is what preserves D10
//! strict-serializability once `seal()` drops the write lock early.
//!
//! # Publish order == seal order (the P0 correctness invariant)
//!
//! `seal()` consumes the [`crate::WriteTxn`], so the write lock drops as it
//! returns — *before* the caller enqueues the bundle. Two sessions can seal in
//! lock order (A then B) yet `send()` in the opposite order, so raw channel
//! arrival order is **not** seal order. To publish in the correct total order
//! anyway, each publishable unit is stamped with a strictly-monotonic `seal_seq`
//! **under the write lock**, and the committer publishes strictly in ascending
//! `seal_seq` via a reorder buffer ([`run_committer`]). Channel arrival order
//! only governs *when* an item reaches the buffer, never the order it publishes.
//! **This is a new, load-bearing, NOT type-enforced invariant** — a second
//! committer or a second `ArcSwap` writer anywhere would silently break
//! serializability (see the v1.2 design §4 "the one honest shift"). Every
//! snapshot publisher routes here; the rerouting completeness is grep-gated and
//! load-bearing.
//!
//! # No committer-held write lock (deadlock surface removed)
//!
//! Compaction also follows seal-and-handover: `SharedGraph::compact` acquires
//! the write lock on the **caller** thread, allocates a `seal_seq`, densifies
//! the live graph, writes it back into `*shared`, releases the lock, and hands
//! the committer a *pre-built* dense snapshot. The committer therefore **never**
//! takes the write lock for any work item — eliminating the
//! send-under-lock/queued-compact deadlock entirely (a session is never
//! simultaneously lock-holding and blocked on the committer).
//!
//! # Group commit (v1.2 multi-writer, BRIEF 2)
//!
//! The committer now drives WAL durability in [`SyncPolicy::OnFlushOnly`]
//! (forced by the builder — see [`crate::SharedGraphBuilder::with_wal`]) and is
//! the **sole fsync caller**. Each loop iteration forms a contiguous-`seal_seq`
//! run of commits ([`crate::committer_batch::drain_contiguous_batch`] — Stage 1
//! append, fsync deferred), runs ONE group flush
//! ([`crate::write_txn::flush_durables`] — Stage 2, the R1
//! fsync-before-publish barrier), then publishes + acks each member in
//! `seal_seq` order ([`crate::write_txn::publish_appended`] — Stage 3/4). The
//! run length is capped at 1 when [`CommitBatching::Off`] (the default) and at
//! N when [`CommitBatching::On`], so **OFF is the degenerate `N=1` case of the
//! identical batched code** — one append + one fsync + one publish + one ack,
//! the same syscalls in the same order as BRIEF 1's `EveryN(1)`. A
//! Snapshot-maintenance work is a hard flush boundary: it is never co-batched
//! (F2 — its replacement snapshot already contains every lower-`seal_seq`
//! commit's mutation, so the pending commit run must be flushed + published
//! first to keep durable-before-visible).
//!
//! [`SyncPolicy::OnFlushOnly`]: crate::SyncPolicy::OnFlushOnly

use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use arc_swap::ArcSwap;

use crate::committer_batch::{
    BatchDrain, BatchLimits, CommitBatching, drain_contiguous_batch, flush_and_publish_batch,
};
use crate::durable_provider::DurableProvider;
use crate::error::{GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::index_provider::IndexProvider;
use crate::write_txn::{CommitOutcome, SealedCommit};

/// Bound on the inbound work queue (global back-pressure). A full channel
/// blocks the enqueuing session — natural global back-pressure with no
/// semaphore. Sized generously so steady-state sessions never block on a
/// healthy committer, while still bounding unbounded fan-in memory.
const WORK_CHANNEL_CAPACITY: usize = 1024;

/// Work submitted to the committer thread, each tagged with its publish-order
/// `seal_seq` (allocated under the write lock by the caller).
///
/// `Commit` carries a fully-built, frozen [`SealedCommit`] (no lock, no graph
/// reference): the committer never re-validates, re-allocates ids, or re-applies
/// a change list. Snapshot-maintenance variants carry a *pre-built* replacement
/// snapshot (built on the caller thread under the lock, like a commit) — the
/// committer never touches the write lock.
///
/// Index DDL is **not** a distinct variant: `create_property_index_named` /
/// `drop_property_index` build + `seal()` their `WriteTxn` on the caller thread
/// (releasing the lock) exactly like any other write, then submit a
/// `Work::Commit`.
pub(crate) enum Work {
    /// Publish a pre-sealed commit (the common path: autocommit, explicit-txn
    /// terminal COMMIT, and index DDL).
    Commit {
        sealed: SealedCommit,
        reply: SyncSender<GraphResult<CommitOutcome>>,
    },
    /// Publish a pre-built dense compacted snapshot. Built + written into
    /// `*shared` on the caller thread under the lock; the committer only swaps
    /// the [`ArcSwap`] cell in `seal_seq` order.
    Compact {
        seal_seq: u64,
        dense: Arc<SeleneGraph>,
        report: crate::CompactionReport,
        reply: SyncSender<GraphResult<crate::CompactionReport>>,
    },
    /// Publish a pre-built snapshot with rebuilt vector indexes. This is pure
    /// derived-state reclamation: no WAL append, no schema bump, no provider
    /// fan-out.
    VectorIndexRebuild {
        seal_seq: u64,
        rebuilt: Arc<SeleneGraph>,
        report: crate::VectorIndexRebuildReport,
        reply: SyncSender<GraphResult<crate::VectorIndexRebuildReport>>,
    },
}

impl Work {
    /// The publish-order key under which the reorder buffer releases this item.
    pub(crate) fn seal_seq(&self) -> u64 {
        match self {
            Work::Commit { sealed, .. } => sealed.seal_seq,
            Work::Compact { seal_seq, .. } => *seal_seq,
            Work::VectorIndexRebuild { seal_seq, .. } => *seal_seq,
        }
    }

    /// Return true for WAL-free snapshot replacement work.
    pub(crate) fn is_snapshot_maintenance(&self) -> bool {
        matches!(self, Work::Compact { .. } | Work::VectorIndexRebuild { .. })
    }
}

/// Long-lived Arc handles the committer thread needs to publish.
///
/// These are clones of the [`crate::SharedGraph`] internals. The committer owns
/// them for its whole life; it is the only thread that calls `snapshot.store`.
/// It does **not** hold the write lock (`shared`) — compaction builds on the
/// caller thread — so no `RwLock` handle is needed here.
pub(crate) struct CommitterHandles {
    /// The published-snapshot cell. The committer is its sole writer.
    pub(crate) snapshot: Arc<ArcSwap<SeleneGraph>>,
    /// Plan-cache schema epoch, bumped strictly after `snapshot.store`.
    pub(crate) schema_version: Arc<AtomicU64>,
    /// Fan-out (no-op in production) providers. Shares the construction-time
    /// frozen registry allocation with [`crate::SharedGraph`].
    pub(crate) providers: Arc<[Arc<dyn IndexProvider>]>,
    /// Commit-critical durable providers (WAL). The committer is their sole
    /// `write_commit`/`flush` caller, which is what makes the BRIEF 2
    /// `OnFlushOnly` toggle committer-exclusive.
    pub(crate) durable_providers: Vec<Arc<dyn DurableProvider>>,
    /// Group-commit policy (BRIEF 2). `Off` (the default) caps each drained run
    /// at one commit ⇒ one append + one fsync per commit (BRIEF-1 behavior);
    /// `On` coalesces a contiguous run into one group flush.
    pub(crate) batching: CommitBatching,
}

/// Handle to the per-graph committer thread, owned by [`crate::SharedGraph`].
///
/// Cloned into every [`crate::WriteTxn`] so `commit()`/`commit_with_principal()`
/// can seal-and-submit without a back-reference to the graph. Dropping the last
/// handle closes the inbound channel; the committer thread then drains and
/// exits, and its [`JoinHandle`] is joined by [`SharedGraph`]'s `Drop`.
#[derive(Clone)]
pub(crate) struct Committer {
    sender: SyncSender<Work>,
    /// Set true if the committer thread died (panic) or a post-seal commit
    /// failed durably. Subsequent submits fail fast with [`GraphError::Durable`]
    /// instead of blocking forever on a `recv()` whose `SyncSender` was dropped,
    /// or trusting an in-memory graph that diverged from the published snapshot.
    poisoned: Arc<std::sync::atomic::AtomicBool>,
    /// Strictly-monotonic publish-order allocator. Each `seal()` / `compact()`
    /// takes the next value **under the write lock**, so the sequence order is
    /// the lock-acquisition (total) order. The committer publishes in this
    /// order via its reorder buffer.
    next_seal_seq: Arc<AtomicU64>,
}

/// Owner-side committer state held by [`crate::SharedGraph`]: the canonical
/// submit handle plus the join handle so the thread is shut down cleanly on drop.
///
/// The canonical [`SyncSender`] lives here in an `Option`; [`Self::handle`]
/// hands out cheap clones to each [`WriteTxn`](crate::WriteTxn). On drop the
/// canonical sender is taken (dropped) first; once every `WriteTxn`-held clone
/// is also gone (they borrow `&SharedGraph`, so they are dropped before
/// `SharedGraph` itself), the channel disconnects, the committer's `recv()`
/// returns `Err`, and the loop exits — then we join the thread.
pub(crate) struct CommitterThread {
    /// The canonical sender — the single structural sender owned here. Cloned
    /// for each `WriteTxn`. Taken (dropped) first on shutdown.
    sender: Option<SyncSender<Work>>,
    poisoned: Arc<std::sync::atomic::AtomicBool>,
    next_seal_seq: Arc<AtomicU64>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl CommitterThread {
    /// Spawn the committer thread for a graph and return its owner-side handle.
    pub(crate) fn spawn(handles: CommitterHandles) -> Self {
        let (sender, receiver) = sync_channel::<Work>(WORK_CHANNEL_CAPACITY);
        let poisoned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // seal_seq starts at 0; the committer's reorder buffer expects the first
        // published item to carry seal_seq 0 (its `next_publish_seq` init).
        let next_seal_seq = Arc::new(AtomicU64::new(0));
        let thread_poisoned = Arc::clone(&poisoned);
        let join = std::thread::Builder::new()
            .name("selene-committer".to_owned())
            .spawn(move || run_committer(receiver, handles, &thread_poisoned))
            .expect("committer thread spawns");
        Self {
            sender: Some(sender),
            poisoned,
            next_seal_seq,
            join: Mutex::new(Some(join)),
        }
    }

    /// Hand out a cheaply-cloneable submit handle bound to this committer.
    pub(crate) fn handle(&self) -> Committer {
        Committer {
            sender: self.sender.clone().expect("committer sender live"),
            poisoned: Arc::clone(&self.poisoned),
            next_seal_seq: Arc::clone(&self.next_seal_seq),
        }
    }
}

impl Drop for CommitterThread {
    fn drop(&mut self) {
        // Drop the canonical sender so the channel closes once every
        // WriteTxn-held clone is also dropped (those borrow &SharedGraph and so
        // are already gone when SharedGraph drops). The committer's recv() then
        // returns Err and the loop exits.
        self.sender = None;
        if let Some(join) = self.join.lock().expect("committer join lock").take() {
            let _ = join.join();
        }
    }
}

impl Committer {
    /// Allocate the next strictly-monotonic publish-order key.
    ///
    /// MUST be called while holding the write lock (in `seal()` / `compact()`),
    /// so the allocation order equals lock-acquisition order. `Relaxed` is sound
    /// for the counter itself: the cross-thread ordering comes from the write
    /// lock's release/acquire, not from this atomic.
    pub(crate) fn next_seal_seq(&self) -> u64 {
        self.next_seal_seq.fetch_add(1, Ordering::Relaxed)
    }

    /// Seal-and-submit a commit, blocking until it is durable + visible.
    ///
    /// The caller MUST have released the write lock (i.e. `sealed` came from a
    /// consumed [`crate::WriteTxn`]) **before** calling this — `seal()` does
    /// exactly that. The committer publishes strictly in `sealed.seal_seq`
    /// order, so channel arrival order (which can differ from seal order) does
    /// not affect the published total order.
    pub(crate) fn submit_commit(&self, sealed: SealedCommit) -> GraphResult<CommitOutcome> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(committer_dead());
        }
        let (reply_tx, reply_rx) = sync_channel::<GraphResult<CommitOutcome>>(1);
        self.sender
            .send(Work::Commit {
                sealed,
                reply: reply_tx,
            })
            .map_err(|_| committer_dead())?;
        // BLOCKS until the committer publishes and acks, so a session never
        // observes its own commit before linearization.
        reply_rx.recv().map_err(|_| committer_dead())?
    }

    #[cfg(test)]
    pub(crate) fn submit_commit_async_for_test(
        &self,
        sealed: SealedCommit,
    ) -> GraphResult<Receiver<GraphResult<CommitOutcome>>> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(committer_dead());
        }
        let (reply_tx, reply_rx) = sync_channel::<GraphResult<CommitOutcome>>(1);
        self.sender
            .send(Work::Commit {
                sealed,
                reply: reply_tx,
            })
            .map_err(|_| committer_dead())?;
        Ok(reply_rx)
    }

    /// Submit a pre-built dense compacted snapshot, blocking until the committer
    /// publishes it (in `seal_seq` order) or reports an error.
    ///
    /// The dense graph is built + written into `*shared` on the caller thread
    /// under the write lock (see [`crate::SharedGraph::compact`]); the committer
    /// only swaps the `ArcSwap` cell. Because the `seal_seq` was allocated under
    /// the same lock, a compact can never be reordered ahead of an
    /// earlier-sealed commit, so the published snapshot never regresses to a
    /// stale (non-dense) layout (P1 fix).
    pub(crate) fn submit_compact(
        &self,
        seal_seq: u64,
        dense: Arc<SeleneGraph>,
        report: crate::CompactionReport,
    ) -> GraphResult<crate::CompactionReport> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(committer_dead());
        }
        let (reply_tx, reply_rx) = sync_channel::<GraphResult<crate::CompactionReport>>(1);
        self.sender
            .send(Work::Compact {
                seal_seq,
                dense,
                report,
                reply: reply_tx,
            })
            .map_err(|_| committer_dead())?;
        reply_rx.recv().map_err(|_| committer_dead())?
    }

    /// Submit a pre-built snapshot with rebuilt vector indexes.
    pub(crate) fn submit_vector_index_rebuild(
        &self,
        seal_seq: u64,
        rebuilt: Arc<SeleneGraph>,
        report: crate::VectorIndexRebuildReport,
    ) -> GraphResult<crate::VectorIndexRebuildReport> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(committer_dead());
        }
        let (reply_tx, reply_rx) = sync_channel::<GraphResult<crate::VectorIndexRebuildReport>>(1);
        self.sender
            .send(Work::VectorIndexRebuild {
                seal_seq,
                rebuilt,
                report,
                reply: reply_tx,
            })
            .map_err(|_| committer_dead())?;
        reply_rx.recv().map_err(|_| committer_dead())?
    }
}

/// Error returned to every waiter when the committer thread is gone (panicked
/// or shutting down). Maps to GQLSTATUS `5GQL0` like any durable failure.
pub(crate) fn committer_dead() -> GraphError {
    GraphError::Durable {
        reason: "commit thread is no longer running; the graph must be reopened".to_owned(),
    }
}

/// Committer thread entry point: drain [`Work`] into a reorder buffer and
/// publish strictly in `seal_seq` order, batching contiguous commits into one
/// group fsync (v1.2 multi-writer, BRIEF 2).
///
/// Items can arrive out of seal order (the lock drops inside `seal()`, before
/// the caller's `send`). The committer buffers each arrival keyed by `seal_seq`
/// and only publishes the contiguous run starting at `next_publish_seq`. This
/// makes publish order == seal order == lock-acquisition order regardless of
/// channel arrival order — the P0 publish-ordering invariant.
///
/// Each contiguous-run pass:
/// 1. If the head is snapshot-maintenance work, publish it **solo** (store the
///    replacement Arc, no append/flush/schema-bump/fan-out) — a hard flush
///    boundary (F2).
/// 2. Otherwise form the contiguous commit run
///    ([`drain_contiguous_batch`] — append, fsync deferred), then
///    [`flush_and_publish_batch`] (ONE group flush == the R1 barrier, then
///    publish + ack each member in `seal_seq` order).
///
/// With [`CommitBatching::Off`] each run is capped at one commit, so this is the
/// degenerate `N=1` case of the identical code — one append + one fsync + one
/// publish + one ack per commit (BRIEF-1 behavior).
fn run_committer(
    receiver: Receiver<Work>,
    handles: CommitterHandles,
    poisoned: &Arc<std::sync::atomic::AtomicBool>,
) {
    // The seal_seq the next publish must carry. Allocation starts at 0, so the
    // first published item is seal_seq 0; thereafter strictly +1 with no gaps
    // (an aborted/cancelled seal never consumes a seal_seq — see
    // `WriteTxn::seal`, which allocates only after every fallible step).
    let mut next_publish_seq: u64 = 0;
    let mut reorder: BTreeMap<u64, Work> = BTreeMap::new();
    let limits = BatchLimits::resolve(handles.batching);
    loop {
        // Block for the next arrival. Channel-closed => owner dropped => exit.
        // A clean shutdown drops the canonical sender only after every
        // WriteTxn-held clone is gone, so no in-flight commit can still be
        // mid-seal when the channel closes; any buffered-but-unpublishable item
        // (a gap that will never fill) is dropped, which Errs its waiter via the
        // dropped reply sender — correct, since shutdown means no more commits.
        let work = match receiver.recv() {
            Ok(work) => work,
            Err(_) => return,
        };
        reorder.insert(work.seal_seq(), work);

        // Drain every contiguous run now available, in seal_seq order.
        while reorder.contains_key(&next_publish_seq) {
            // F2: snapshot maintenance at head publishes solo (empty batch by
            // construction — all lower seqs are already durable + visible, so
            // this needs zero flush calls). It is never co-batched with commits.
            if reorder
                .get(&next_publish_seq)
                .is_some_and(Work::is_snapshot_maintenance)
            {
                let Some(work) = reorder.remove(&next_publish_seq) else {
                    unreachable!("checked maintenance work at next_publish_seq above");
                };
                publish_snapshot_maintenance(work, poisoned, &handles);
                next_publish_seq += 1;
                if poisoned.load(Ordering::Acquire) {
                    drain_buffer_with_error(&mut reorder);
                    return;
                }
                continue;
            }

            // Phase 1: form the contiguous commit run (append, fsync deferred).
            match drain_contiguous_batch(
                &receiver,
                &mut reorder,
                &mut next_publish_seq,
                limits,
                &handles,
                poisoned,
            ) {
                BatchDrain::Run { batch } => {
                    // Phase 2 (R1 barrier) + Phase 3/4 (publish + ack). Returns
                    // true when poisoned + drained ⇒ stop.
                    if flush_and_publish_batch(batch, &mut reorder, &handles, poisoned) {
                        return;
                    }
                }
                BatchDrain::AppendFailed { appended } => {
                    // A Stage-1 append failed: the failed waiter was already
                    // Err'd inside drain_contiguous_batch; here we Err every
                    // already-appended member (their unflushed bytes are correct
                    // to lose on reopen), drain the buffer, and exit. Nothing in
                    // the run was flushed or published.
                    crate::committer_batch::ack_appended_with_error(appended);
                    drain_buffer_with_error(&mut reorder);
                    return;
                }
            }
        }
    }
}

/// Publish WAL-free snapshot maintenance (F2 hard flush boundary): store the
/// replacement Arc only — no append, no flush, no schema-bump, no fan-out.
/// Wrapped in `catch_unwind`; a `store` panic poisons (see
/// [`unwrap_protected`]). All lower `seal_seq` commits are already durable +
/// visible by the time maintenance reaches head, so it needs zero flush calls.
fn publish_snapshot_maintenance(
    work: Work,
    poisoned: &Arc<std::sync::atomic::AtomicBool>,
    handles: &CommitterHandles,
) {
    match work {
        Work::Compact {
            seal_seq: _,
            dense,
            report,
            reply,
        } => {
            let result =
                run_protected(|| publish_replacement_snapshot(&dense, &handles.snapshot, report));
            let result = unwrap_protected(result, poisoned);
            let _ = reply.send(result);
        }
        Work::VectorIndexRebuild {
            seal_seq: _,
            rebuilt,
            report,
            reply,
        } => {
            let result =
                run_protected(|| publish_replacement_snapshot(&rebuilt, &handles.snapshot, report));
            let result = unwrap_protected(result, poisoned);
            let _ = reply.send(result);
        }
        Work::Commit { .. } => {
            unreachable!("publish_snapshot_maintenance is never called with Work::Commit");
        }
    }
}

/// Publish a pre-built replacement snapshot. Pure space reclamation: no WAL
/// append, no schema bump, no fan-out — only the `ArcSwap` swap.
///
/// The replacement graph's structural consistency was already verified or
/// rebuilt on the caller thread, **before** it was written into `*shared` — so a
/// broken replacement never reaches this point. Re-asserting here, after the
/// store, would risk the same "returns-Err-but-actually-published" inversion
/// the commit path avoids (P2), for zero added coverage. Wrapped in
/// `catch_unwind` by the caller only so a `store` panic still poisons rather
/// than aborts the committer thread.
fn publish_replacement_snapshot<T>(
    replacement: &Arc<SeleneGraph>,
    snapshot: &ArcSwap<SeleneGraph>,
    report: T,
) -> GraphResult<T> {
    snapshot.store(Arc::clone(replacement));
    Ok(report)
}

/// Fail every buffered waiter with `committer_dead` so no reply `SyncSender` is
/// dropped silently (which would hang its `recv()` with a `RecvError`). Called
/// once the committer is poisoned and about to exit.
pub(crate) fn drain_buffer_with_error(reorder: &mut BTreeMap<u64, Work>) {
    for (_, work) in std::mem::take(reorder) {
        match work {
            Work::Commit { reply, .. } => {
                let _ = reply.send(Err(committer_dead()));
            }
            Work::Compact { reply, .. } => {
                let _ = reply.send(Err(committer_dead()));
            }
            Work::VectorIndexRebuild { reply, .. } => {
                let _ = reply.send(Err(committer_dead()));
            }
        }
    }
}

/// Run a committer body inside `catch_unwind`. parking_lot does not poison, so
/// a panic leaves locks usable; the engine is poisoned at a higher level
/// instead (no further commits trusted).
pub(crate) fn run_protected<T>(
    body: impl FnOnce() -> GraphResult<T>,
) -> Result<GraphResult<T>, Box<dyn std::any::Any + Send>> {
    std::panic::catch_unwind(AssertUnwindSafe(body))
}

/// Convert a `catch_unwind` result into a `GraphResult`, poisoning the committer
/// on a panic **or a returned error** so subsequent submits fail fast.
///
/// A returned `Err` from a publish is a *post-seal* failure: `seal()` already
/// wove the commit's mutation into `*shared` (and a later seal may have forked
/// off it), so the live in-memory graph cannot be surgically rolled back to
/// exclude only the failed commit. Poisoning is therefore the only consistent
/// recovery — the durable WAL never received the failed entry, so a reopen
/// (recovery) heals the divergence. This restores the pre-v1.2 invariant that a
/// commit reporting `Err` never leaks into the published snapshot or any later
/// commit's baseline (the failed-`write_commit` regression, P0).
pub(crate) fn unwrap_protected<T>(
    result: Result<GraphResult<T>, Box<dyn std::any::Any + Send>>,
    poisoned: &Arc<std::sync::atomic::AtomicBool>,
) -> GraphResult<T> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => {
            // Post-seal durable failure: the in-memory `*shared` already
            // advanced past this commit and cannot be unwound, so poison.
            poisoned.store(true, Ordering::Release);
            tracing::error!(
                error = %error,
                "selene-graph: commit failed after seal; engine poisoned, reopen required",
            );
            Err(error)
        }
        Err(payload) => {
            poisoned.store(true, Ordering::Release);
            let description = crate::panic_payload::describe(&payload);
            tracing::error!(
                payload = %description,
                "selene-graph: commit thread panicked; engine poisoned, reopen required",
            );
            // Open-risk #2 (split-brain): a panic between seal() and store()
            // can leave the live guard-Arc and the published snapshot
            // divergent for that commit. We cannot reconcile in-process; the
            // engine is poisoned and a reopen (recovery from the durable WAL,
            // which never saw the un-appended commit) restores consistency.
            Err(GraphError::Durable {
                reason: format!("commit thread panicked: {description}"),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use selene_core::{Change, GraphId, HlcTimestamp, LabelSet, PropertyMap};

    use crate::SharedGraph;
    use crate::durable_provider::DurableProvider;
    use crate::error::GraphError;
    use crate::index_provider::{ProviderError, ProviderTag};

    fn db_string(value: &str) -> selene_core::DbString {
        selene_core::db_string(value).expect("string fits DB string cap")
    }

    /// Durable provider whose `write_commit` panics, killing the committer's
    /// publish body (the panic propagates to the committer's `catch_unwind`,
    /// which poisons the committer). Used to drive the committer-death path
    /// deterministically in both debug and release builds.
    struct PanicOnWriteCommit;

    impl DurableProvider for PanicOnWriteCommit {
        fn provider_tag(&self) -> ProviderTag {
            ProviderTag(*b"BOOM")
        }
        fn write_commit(
            &self,
            _principal: Option<&Arc<[u8]>>,
            _changes: &[Change],
            _timestamp: HlcTimestamp,
        ) -> Result<u64, ProviderError> {
            panic!("synthetic committer-body panic in write_commit");
        }
    }

    fn graph_with_panicking_durable(id: u64) -> SharedGraph {
        SharedGraph::from_graph_with_core_and_durables(
            crate::SeleneGraph::new(GraphId::new(id)),
            Vec::new(),
            vec![Arc::new(PanicOnWriteCommit) as Arc<dyn DurableProvider>],
            None,
            None,
            crate::committer_batch::CommitBatching::Off,
        )
        .expect("graph builds with synthetic durable provider")
    }

    /// Durable provider that **returns `Err`** (does not panic) on its FIRST
    /// `write_commit`, then succeeds (ascending sequences) for every subsequent
    /// call.
    ///
    /// This isolates the *poison* effect from "the provider just always fails":
    /// the first commit fails post-seal (its mutation is already woven into
    /// `*shared`). Without poisoning, a healthy engine would let the SECOND
    /// commit succeed — and that second commit would fork from the diverged
    /// `*shared` (carrying the failed commit's leaked node) and publish BOTH
    /// nodes. Poisoning makes the second commit fail fast, so the leaked node can
    /// never reach the published snapshot (the P0 failed-`write_commit`
    /// regression).
    struct FailFirstWriteCommit {
        calls: AtomicU64,
    }

    impl DurableProvider for FailFirstWriteCommit {
        fn provider_tag(&self) -> ProviderTag {
            ProviderTag(*b"FRST")
        }
        fn write_commit(
            &self,
            _principal: Option<&Arc<[u8]>>,
            _changes: &[Change],
            _timestamp: HlcTimestamp,
        ) -> Result<u64, ProviderError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Err(ProviderError::Inconsistent {
                    reason: "synthetic first-commit durable failure".to_owned(),
                })
            } else {
                Ok(n)
            }
        }
    }

    fn graph_with_fail_first_durable(id: u64) -> SharedGraph {
        SharedGraph::from_graph_with_core_and_durables(
            crate::SeleneGraph::new(GraphId::new(id)),
            Vec::new(),
            vec![Arc::new(FailFirstWriteCommit {
                calls: AtomicU64::new(0),
            }) as Arc<dyn DurableProvider>],
            None,
            None,
            crate::committer_batch::CommitBatching::Off,
        )
        .expect("graph builds with synthetic fail-first durable provider")
    }

    #[test]
    fn cancel_cutline_in_seal_rolls_back_with_no_burned_state() {
        // The BRIEF-117 cut-line is sampled inside seal(), under the write lock,
        // before Drop is disarmed. An already-set token must abort the commit
        // with Cancelled AND roll the in-memory graph back (via Drop) so NOTHING
        // is left advanced — not the published snapshot, not the live RwLock
        // graph, not the WAL — exactly as an aborted transaction would leave it.
        let dir = std::env::temp_dir().join(format!(
            "selene-committer-cancel-{}-{:?}",
            std::process::id(),
            Instant::now()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wal_path = dir.join(selene_persist::DEFAULT_WAL_FILE_NAME);
        let shared = SharedGraph::builder(GraphId::new(91_001))
            .with_wal(&wal_path, selene_persist::WalConfig::default())
            .unwrap()
            .build()
            .unwrap();

        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        // An already-set token makes seal() return Cancelled and roll back.
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let err = match txn.seal(None, Some(&flag)) {
            Ok(_) => panic!("pre-publish cancel must return Err from seal"),
            Err(err) => err,
        };
        assert!(matches!(err, GraphError::Cancelled), "got {err:?}");
        assert_eq!(err.gqlstatus(), "5GQL2");

        // Nothing published, nothing appended, and the LIVE RwLock graph is
        // rolled back too (not just the published ArcSwap): published snapshot
        // and the locked graph agree, both at the pre-commit baseline.
        assert_eq!(shared.read().node_count(), 0);
        assert_eq!(shared.read().meta.generation, 0);
        assert_eq!(shared.locked_generation_for_test(), 0);
        assert_eq!(
            shared.locked_arc_ptr_for_test(),
            Arc::as_ptr(&shared.read()),
            "live RwLock graph and published snapshot are the same Arc after a \
             cancelled seal — no divergence",
        );

        // A subsequent uncancelled commit gets WAL seq 1 — the cancelled commit
        // burned no durable sequence (it never appended) and no seal_seq.
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        let outcome = txn.commit().unwrap();
        assert_eq!(outcome.durable_at, Some(1));
        assert_eq!(outcome.generation, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_token_unset_at_seal_proceeds_and_is_irrevocable() {
        // When the cut-line samples the token as false it proceeds; flipping the
        // token afterward cannot revoke the published commit.
        let shared = SharedGraph::new(GraphId::new(91_002));
        let mut txn = shared.begin_write();
        let id = txn
            .mutator()
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sealed = txn.seal(None, Some(&flag)).expect("uncancelled seal");
        let outcome = shared
            .submit_sealed_for_test(sealed)
            .expect("uncancelled commit publishes");
        // Flip after the fact — no effect, the commit already linearized.
        flag.store(true, std::sync::atomic::Ordering::Release);
        assert_eq!(outcome.generation, 1);
        assert!(shared.read().is_node_alive(id));
    }

    #[test]
    fn committer_panic_poisons_and_fails_all_waiters_without_hanging() {
        // The first commit's write_commit panics on the committer thread. The
        // catch_unwind poisons the committer and Errs THIS waiter (never drops
        // the reply SyncSender → never a silent RecvError hang). Subsequent
        // submits fail fast within a bounded deadline.
        let shared = graph_with_panicking_durable(91_004);

        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(db_string("L")), PropertyMap::new())
            .unwrap();
        let first = txn.commit();
        assert!(
            matches!(first, Err(GraphError::Durable { .. })),
            "the panicking commit reports a Durable error, got {first:?}"
        );

        // Subsequent commit fails fast (poisoned) — no hang.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        let second = txn.commit();
        assert!(Instant::now() < deadline, "post-poison commit did not hang");
        assert!(
            matches!(second, Err(GraphError::Durable { .. })),
            "post-poison commit fails fast, got {second:?}"
        );
    }

    #[test]
    fn concurrent_panicking_commits_all_err_without_hanging() {
        // Many panicking commits driven concurrently: the first to reach the
        // committer panics + poisons; every other waiter (whether buffered in
        // the reorder buffer, in-channel, or arriving post-poison) must receive
        // Err in bounded time — never a dropped SyncSender → silent RecvError
        // hang. (Was misnamed `mid_batch_panic`; with cap-1 there is no >1 batch
        // — this exercises the poison-fan-out + drain-buffer paths.)
        let shared = Arc::new(graph_with_panicking_durable(91_005));
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut handles = Vec::new();
        for _ in 0..8 {
            let shared = Arc::clone(&shared);
            handles.push(std::thread::spawn(move || {
                let mut txn = shared.begin_write();
                txn.mutator()
                    .create_node(LabelSet::new(), PropertyMap::new())
                    .unwrap();
                txn.commit()
            }));
        }
        for h in handles {
            let result = h.join().expect("waiter thread did not panic");
            assert!(
                result.is_err(),
                "every waiter behind a poisoned committer gets Err, got {result:?}"
            );
        }
        assert!(Instant::now() < deadline, "no waiter hung after the panic");
    }

    #[test]
    fn returned_write_commit_err_poisons_so_failed_commit_never_leaks() {
        // P0 (failed-write_commit regression): a durable provider RETURNS Err
        // (does not panic) on the first commit, AFTER seal() already wove the
        // mutation into `*shared`. The committer must NOT publish, must report
        // Err, and must POISON so the diverged in-memory graph is never trusted.
        //
        // The provider succeeds on the SECOND commit, so this distinguishes the
        // poison fix from "provider always fails": WITHOUT poison, the second
        // commit would succeed and publish a snapshot forked from the diverged
        // `*shared` (which still carries the first, never-persisted node) —
        // leaking it into the published state. WITH poison, the second commit
        // fails fast and the leaked node never becomes visible.
        let shared = graph_with_fail_first_durable(91_006);

        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(db_string("L")), PropertyMap::new())
            .unwrap();
        let first = txn.commit();
        assert!(
            matches!(first, Err(GraphError::Durable { .. })),
            "a returned write_commit Err surfaces as Durable, got {first:?}"
        );

        // Not visible: the published snapshot never advanced past the failure.
        assert_eq!(shared.read().node_count(), 0);
        assert_eq!(shared.read().meta.generation, 0);

        // Engine poisoned: the next commit fails fast even though the provider
        // would now succeed — so the diverged `*shared` (carrying the leaked
        // first node) can NEVER reach the published snapshot. Bounded; no hang.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        let second = txn.commit();
        assert!(Instant::now() < deadline, "post-poison commit did not hang");
        assert!(
            matches!(second, Err(GraphError::Durable { .. })),
            "post-poison commit fails fast (engine poisoned), got {second:?}"
        );

        // The leaked first node never became visible — the regression's blast
        // radius (a never-persisted node silently published by a later commit)
        // is closed.
        assert_eq!(
            shared.read().node_count(),
            0,
            "the failed commit's node never leaked into the published snapshot",
        );
    }

    #[test]
    fn reorder_buffer_publishes_in_seal_order_not_arrival_order() {
        // P0 (publish-order == seal-order): force the exact reorder race. Seal A
        // first (seal_seq 0) and B second (seal_seq 1) under the lock — B forks
        // off A's frozen Arc, so A's snapshot does NOT contain B's node and B's
        // gen is strictly higher. Then submit B's bundle to the committer FIRST
        // (reverse of seal order) and A's SECOND. A correct reorder-buffer
        // committer publishes A (seq 0) then B (seq 1); the final published
        // snapshot is B's gen-2 graph containing BOTH nodes. A raw-FIFO committer
        // would publish B then A, leaving the final snapshot = A's gen-1 graph
        // missing B's node and regressing the generation — turning this RED.
        let shared = Arc::new(SharedGraph::new(GraphId::new(91_007)));

        // Seal A under the lock (seal_seq 0); lock released as seal() returns.
        let mut txn_a = shared.begin_write();
        let a = txn_a
            .mutator()
            .create_node(LabelSet::single(db_string("A")), PropertyMap::new())
            .unwrap();
        let sealed_a = txn_a.seal(None, None).expect("A seals");

        // Seal B under the lock (seal_seq 1); B's guard_mut forks off A's Arc.
        let mut txn_b = shared.begin_write();
        let b = txn_b
            .mutator()
            .create_node(LabelSet::single(db_string("B")), PropertyMap::new())
            .unwrap();
        let sealed_b = txn_b.seal(None, None).expect("B seals");

        // Submit B FIRST (reverse of seal order) on a background thread — it
        // sends B then blocks in recv until B publishes. The committer buffers B
        // (waiting for seq 0). A short yield gives B's send time to land, then
        // we submit A, which unblocks the contiguous publish of A then B.
        let shared_b = Arc::clone(&shared);
        let b_thread = std::thread::spawn(move || {
            shared_b
                .submit_sealed_for_test(sealed_b)
                .expect("B publishes after A")
        });
        // Yield so B's send reaches the committer's reorder buffer before A's.
        for _ in 0..1000 {
            std::thread::yield_now();
        }
        let outcome_a = shared
            .submit_sealed_for_test(sealed_a)
            .expect("A publishes");
        let outcome_b = b_thread.join().expect("B thread did not panic");

        // Generations reflect seal order regardless of submit order.
        assert_eq!(outcome_a.generation, 1, "A is seal_seq 0 ⇒ generation 1");
        assert_eq!(outcome_b.generation, 2, "B is seal_seq 1 ⇒ generation 2");

        // The FINAL published snapshot is B's (the higher seal_seq), and it
        // contains BOTH nodes — the publish order was A then B, not B then A.
        let snap = shared.read();
        assert_eq!(
            snap.meta.generation, 2,
            "final published gen == max seal_seq"
        );
        assert!(
            snap.is_node_alive(a),
            "A's node survived in the final snapshot"
        );
        assert!(
            snap.is_node_alive(b),
            "B's node is present in the final snapshot"
        );
        assert_eq!(snap.node_count(), 2);
    }

    #[test]
    fn compact_cannot_clobber_an_earlier_sealed_commit() {
        // P1 (compact-vs-commit reorder / lost reclamation): seal a commit A
        // (seal_seq 0) WITHOUT publishing it, then run compact() — which takes
        // its seal_seq (1) under the lock, AFTER A's. Submit the compact's
        // publish FIRST and A SECOND. Because the committer publishes in seal_seq
        // order, A (seq 0) publishes before the compact (seq 1), so the dense
        // compacted snapshot is the FINAL published state — its reclamation is
        // never clobbered by A's stale pre-compaction frozen snapshot.
        let shared = Arc::new(SharedGraph::new(GraphId::new(91_008)));
        // Seed reclaimable holes: create then delete.
        {
            let mut txn = shared.begin_write();
            let mut ids = Vec::new();
            for _ in 0..20 {
                ids.push(
                    txn.mutator()
                        .create_node(LabelSet::single(db_string("S")), PropertyMap::new())
                        .unwrap(),
                );
            }
            txn.commit().unwrap();
            let mut txn = shared.begin_write();
            for id in &ids {
                txn.mutator().delete_node(*id).unwrap();
            }
            txn.commit().unwrap();
        }

        // Seal A (a fresh node) but do not submit yet — seal_seq is the next.
        let mut txn_a = shared.begin_write();
        let a = txn_a
            .mutator()
            .create_node(LabelSet::single(db_string("A")), PropertyMap::new())
            .unwrap();
        let sealed_a = txn_a.seal(None, None).expect("A seals");

        // Run compact on a background thread (it seals_seq AFTER A under the lock
        // and then blocks until its publish lands). Yield so compact's enqueue
        // reaches the buffer before A's, then submit A.
        let shared_c = Arc::clone(&shared);
        let compactor = std::thread::spawn(move || shared_c.compact().expect("compaction ok"));
        for _ in 0..1000 {
            std::thread::yield_now();
        }
        let outcome_a = shared
            .submit_sealed_for_test(sealed_a)
            .expect("A publishes");
        let report = compactor.join().expect("compactor did not panic");

        // A published at seal_seq 0 (generation = the third commit).
        assert_eq!(outcome_a.generation, 3);
        // The compaction reclaimed the 20 deleted holes.
        assert!(report.reclaimed_nodes >= 20, "report: {report:?}");

        // The FINAL published snapshot is the dense compacted one: A is present
        // AND the row layout is dense (node row count == live node count == 1).
        // A clobber (raw FIFO) would leave A's non-dense frozen snapshot with the
        // holes intact, so the published store would still carry the 20 dead
        // rows — turning the density assertion RED.
        let snap = shared.read();
        assert!(snap.is_node_alive(a));
        assert_eq!(snap.node_count(), 1, "only A is alive");
        assert_eq!(
            snap.node_store.len(),
            1,
            "published snapshot is dense — the compaction's reclamation was not \
             clobbered by A's stale pre-compaction snapshot",
        );
        snap.assert_indexes_consistent()
            .expect("published snapshot is structurally consistent");
    }
}
