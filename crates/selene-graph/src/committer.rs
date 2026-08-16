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
//! Snapshot-maintenance and coordinated-checkpoint work are hard flush
//! boundaries: neither is co-batched. A replacement snapshot already contains
//! every lower-`seal_seq` mutation, while a checkpoint must serialize only
//! after every lower commit is durable, visible, and applied to providers.
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
/// snapshot (built on the caller thread under the lock, like a commit), while a
/// checkpoint carries the generation captured at its ordered boundary. The
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
    /// Serialize the published graph/provider generation at this ordered
    /// boundary and rotate the owned WAL through the MANIFEST protocol.
    Checkpoint {
        seal_seq: u64,
        generation: u64,
        config: crate::CheckpointConfig,
        reply: SyncSender<GraphResult<crate::CheckpointOutcome>>,
    },
}

impl Work {
    /// The publish-order key under which the reorder buffer releases this item.
    pub(crate) fn seal_seq(&self) -> u64 {
        match self {
            Work::Commit { sealed, .. } => sealed.seal_seq,
            Work::Compact { seal_seq, .. } => *seal_seq,
            Work::VectorIndexRebuild { seal_seq, .. } => *seal_seq,
            Work::Checkpoint { seal_seq, .. } => *seal_seq,
        }
    }

    /// Return true for work that must run alone at an ordered hard boundary.
    pub(crate) fn is_solo(&self) -> bool {
        matches!(
            self,
            Work::Compact { .. } | Work::VectorIndexRebuild { .. } | Work::Checkpoint { .. }
        )
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
    /// Typed CORE provider that owns the live WAL writer used by coordinated
    /// checkpoints.
    pub(crate) core: Arc<crate::CoreProvider>,
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

    /// Submit a coordinated checkpoint at an already-allocated ordered
    /// boundary and wait for its MANIFEST-backed WAL rotation.
    pub(crate) fn submit_checkpoint(
        &self,
        seal_seq: u64,
        generation: u64,
        config: crate::CheckpointConfig,
    ) -> GraphResult<crate::CheckpointOutcome> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(committer_dead());
        }
        let (reply_tx, reply_rx) = sync_channel::<GraphResult<crate::CheckpointOutcome>>(1);
        self.sender
            .send(Work::Checkpoint {
                seal_seq,
                generation,
                config,
                reply: reply_tx,
            })
            .map_err(|_| committer_dead())?;
        reply_rx.recv().map_err(|_| committer_dead())?
    }
}

/// Error returned to every waiter when the committer thread is gone (panicked
/// or shutting down).
///
/// [`GraphError::IndeterminateOutcome`] (GQLSTATUS `40003`), not
/// [`GraphError::Durable`]: a waiter reaching here may have had its record
/// appended, or appended and fsynced, and cannot tell which. See that variant
/// for why an unqualified rollback would be the dangerous lie.
pub(crate) fn committer_dead() -> GraphError {
    GraphError::IndeterminateOutcome {
        reason: "commit thread is no longer running; the graph must be reopened".to_owned(),
    }
}

/// Reclassify a poison-exit error as an indeterminate commit outcome.
///
/// The two poison paths that reply with their own error rather than
/// [`committer_dead`] — a partial-batch append failure and a publish-tail panic
/// — are both reached with bytes possibly already in the WAL, so the outcome is
/// as unknown as any other member's. Their diagnostic text is preserved; only
/// the classification changes.
pub(crate) fn indeterminate(error: GraphError) -> GraphError {
    match error {
        // Already classified; do not nest the reason text.
        indeterminate @ GraphError::IndeterminateOutcome { .. } => indeterminate,
        other => GraphError::IndeterminateOutcome {
            reason: other.to_string(),
        },
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
/// 1. If the head is snapshot-maintenance or checkpoint work, run it **solo**
///    as a hard flush boundary. Maintenance stores its replacement Arc;
///    checkpointing serializes providers and rotates the WAL.
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
            // Snapshot replacement and checkpoint work publish solo. All lower
            // seqs are already durable + visible, and no higher commit can be
            // appended across this hard boundary.
            if reorder.get(&next_publish_seq).is_some_and(Work::is_solo) {
                let Some(work) = reorder.remove(&next_publish_seq) else {
                    unreachable!("checked solo work at next_publish_seq above");
                };
                publish_solo(work, poisoned, &handles);
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
                    // already-appended member, drain the buffer, and exit.
                    // Nothing in the run was flushed or published — but their
                    // records ARE written, and a reopen replays them, so each
                    // is acked with committer_dead's IndeterminateOutcome rather
                    // than a rollback that claims they did not happen.
                    crate::committer_batch::ack_appended_with_error(appended);
                    drain_buffer_with_error(&mut reorder);
                    return;
                }
            }
        }
    }
}

/// Execute one solo hard-boundary work item.
///
/// Snapshot maintenance stores only the replacement Arc. Checkpoints serialize
/// the exact provider generation and run the MANIFEST-backed WAL rotation. All
/// lower `seal_seq` commits are already durable and visible when this runs.
fn publish_solo(
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
        Work::Checkpoint {
            seal_seq: _,
            generation,
            config,
            reply,
        } => {
            let execution =
                crate::checkpoint::execute(&handles.core, &handles.providers, generation, config);
            let result = if execution.poison_committer {
                poisoned.store(true, Ordering::Release);
                // The poison bit used to stop here, so the caller received the
                // raw rotation error — an ordinary-looking `Persist(Io(..))` —
                // while the handle it holds is dead. Reclassify so the failure
                // says so, exactly as the commit path does. Every LATER submit
                // already answered `committer_dead()`; this makes the call that
                // caused the poison agree with the ones that follow it.
                execution.result.map_err(indeterminate)
            } else {
                execution.result
            };
            let _ = reply.send(result);
        }
        Work::Commit { .. } => {
            unreachable!("publish_solo is never called with Work::Commit");
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
            Work::Checkpoint { reply, .. } => {
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
            // Reclassified HERE, at the point that sets the poison bit, rather
            // than at each reply site. Poisoning and handing back a raw error is
            // the defect #1087 was filed for, and it happened because the
            // checkpoint arm set the bit and forgot to reclassify. Doing it at
            // the choke point makes that combination unexpressible for every
            // caller that poisons through this function.
            Err(indeterminate(error))
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
            // engine is poisoned and a reopen restores consistency — but what
            // the reopen restores TO depends on where the panic landed. A panic
            // in the publish tail is past the group flush, so that commit is
            // fsynced and recovery replays it; a panic before the append leaves
            // nothing. The caller cannot tell the two apart from here, so the
            // outcome is indeterminate by construction.
            //
            // Built directly rather than through `indeterminate`, which
            // composes the wrapped error's Display and would file a panic under
            // "durable provider failed".
            Err(GraphError::IndeterminateOutcome {
                reason: format!("commit thread panicked: {description}"),
            })
        }
    }
}

#[cfg(test)]
mod tests;
