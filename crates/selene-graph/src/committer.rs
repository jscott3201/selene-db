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
//! single-writer + single-threaded-FIFO discipline is what preserves D10
//! strict-serializability once `seal()` drops the write lock early: publish
//! order is the FIFO order of [`Work`] items, which equals seal order, which
//! equals lock-acquisition order. **This is a new, load-bearing, NOT
//! type-enforced invariant** — a second committer or a second `ArcSwap` writer
//! anywhere would silently break serializability (see the v1.2 design §4 "the
//! one honest shift"). Every snapshot publisher routes here; the rerouting
//! completeness is grep-gated and load-bearing.
//!
//! BRIEF 1 is **durability-neutral**: the WAL stays in `SyncPolicy::EveryN(1)`
//! and the drain cap is `1`, so behavior is identical to the pre-v1.2
//! per-commit fsync — only the threading model changes. BRIEF 2 raises the cap
//! and swaps `EveryN(1)` for `OnFlushOnly` + one group flush per batch.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use arc_swap::ArcSwap;
use parking_lot::RwLock;

use crate::durable_provider::DurableProvider;
use crate::error::{GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::index_provider::IndexProvider;
use crate::write_txn::{CommitOutcome, SealedCommit, publish_sealed};

/// Bound on the inbound work queue (global back-pressure). A full channel
/// blocks the enqueuing session — natural global back-pressure with no
/// semaphore. Sized generously so steady-state sessions never block on a
/// healthy committer, while still bounding unbounded fan-in memory.
const WORK_CHANNEL_CAPACITY: usize = 1024;

/// BRIEF 1 drain cap: process exactly one [`Work::Commit`] per loop turn.
///
/// Batching (raising this cap + swapping `EveryN(1)` for `OnFlushOnly` + one
/// group flush) is BRIEF 2. The drain loop is structured so BRIEF 2 only raises
/// this constant and adds the group-flush stage. Compact items are always batch
/// boundaries regardless of the cap.
const MAX_COMMIT_BATCH: usize = 1;

/// Work submitted to the committer thread.
///
/// `Commit` carries a fully-built, frozen [`SealedCommit`] (no lock, no graph
/// reference): the committer never re-validates, re-allocates ids, or re-applies
/// a change list. `Compact` is the only variant for which the committer touches
/// the write lock itself (it must read the live graph to build the dense
/// result), so it is always a batch boundary.
///
/// Index DDL is **not** a distinct variant: `create_property_index_named` /
/// `drop_property_index` build + `seal()` their `WriteTxn` on the caller thread
/// (releasing the lock) exactly like any other write, then submit a
/// `Work::Commit`. Routing DDL through the same seal path avoids duplicating the
/// build logic on the committer thread and keeps the committer's lock surface to
/// the single `Compact` case (which is what the deadlock invariant below
/// guards).
enum Work {
    /// Publish a pre-sealed commit (the common path: autocommit, explicit-txn
    /// terminal COMMIT, and index DDL).
    Commit {
        sealed: SealedCommit,
        reply: SyncSender<GraphResult<CommitOutcome>>,
    },
    /// Compact the live graph in place and republish. The committer acquires the
    /// write lock itself for this variant — see the deadlock invariant.
    Compact {
        reply: SyncSender<GraphResult<crate::CompactionReport>>,
    },
}

/// Long-lived Arc handles the committer thread needs to publish + compact.
///
/// These are clones of the [`crate::SharedGraph`] internals. The committer owns
/// them for its whole life; it is the only thread that calls `snapshot.store`.
pub(crate) struct CommitterHandles {
    /// The single graph write lock (only taken by the committer for `Compact`).
    pub(crate) shared: Arc<RwLock<Arc<SeleneGraph>>>,
    /// The published-snapshot cell. The committer is its sole writer.
    pub(crate) snapshot: Arc<ArcSwap<SeleneGraph>>,
    /// Plan-cache schema epoch, bumped strictly after `snapshot.store`.
    pub(crate) schema_version: Arc<AtomicU64>,
    /// Fan-out (no-op in production) providers.
    pub(crate) providers: Vec<Arc<dyn IndexProvider>>,
    /// Commit-critical durable providers (WAL). The committer is their sole
    /// `write_commit`/`flush` caller, which is what makes the BRIEF 2
    /// `OnFlushOnly` toggle committer-exclusive.
    pub(crate) durable_providers: Vec<Arc<dyn DurableProvider>>,
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
    /// Set true if the committer thread died (panic). Subsequent submits
    /// fail fast with [`GraphError::Durable`] instead of blocking forever on a
    /// `recv()` whose `SyncSender` was dropped.
    poisoned: Arc<std::sync::atomic::AtomicBool>,
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
    join: Mutex<Option<JoinHandle<()>>>,
}

impl CommitterThread {
    /// Spawn the committer thread for a graph and return its owner-side handle.
    pub(crate) fn spawn(handles: CommitterHandles) -> Self {
        let (sender, receiver) = sync_channel::<Work>(WORK_CHANNEL_CAPACITY);
        let poisoned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_poisoned = Arc::clone(&poisoned);
        let join = std::thread::Builder::new()
            .name("selene-committer".to_owned())
            .spawn(move || run_committer(receiver, handles, &thread_poisoned))
            .expect("committer thread spawns");
        Self {
            sender: Some(sender),
            poisoned,
            join: Mutex::new(Some(join)),
        }
    }

    /// Hand out a cheaply-cloneable submit handle bound to this committer.
    pub(crate) fn handle(&self) -> Committer {
        Committer {
            sender: self.sender.clone().expect("committer sender live"),
            poisoned: Arc::clone(&self.poisoned),
        }
    }

    /// Submit a compaction request and block until the committer publishes the
    /// dense graph or reports an error.
    pub(crate) fn compact(&self) -> GraphResult<crate::CompactionReport> {
        self.handle().submit_compact()
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
    /// Seal-and-submit a commit, blocking until it is durable + visible.
    ///
    /// The caller MUST have released the write lock (i.e. `sealed` came from a
    /// consumed [`crate::WriteTxn`]) **before** calling this. Holding the lock
    /// here would deadlock against a queued `Work::Compact` for which the
    /// committer must acquire the same lock — the load-bearing deadlock
    /// invariant (v1.2 design §3.2).
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
        // BLOCKS until the committer publishes (Stage 3) and acks (Stage 4),
        // so a session never observes its own commit before linearization.
        reply_rx.recv().map_err(|_| committer_dead())?
    }

    fn submit_compact(&self) -> GraphResult<crate::CompactionReport> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(committer_dead());
        }
        let (reply_tx, reply_rx) = sync_channel::<GraphResult<crate::CompactionReport>>(1);
        self.sender
            .send(Work::Compact { reply: reply_tx })
            .map_err(|_| committer_dead())?;
        reply_rx.recv().map_err(|_| committer_dead())?
    }
}

/// Error returned to every waiter when the committer thread is gone (panicked
/// or shutting down). Maps to GQLSTATUS `5GQL0` like any durable failure.
fn committer_dead() -> GraphError {
    GraphError::Durable {
        reason: "commit thread is no longer running; the graph must be reopened".to_owned(),
    }
}

/// Committer thread entry point: drain [`Work`] in FIFO and publish.
fn run_committer(
    receiver: Receiver<Work>,
    handles: CommitterHandles,
    poisoned: &Arc<std::sync::atomic::AtomicBool>,
) {
    // A compaction request can surface while draining a commit batch (mpsc has
    // no un-receive). Stash it here and service it as a boundary immediately
    // after the in-flight commit batch publishes, preserving FIFO order.
    let mut pending_compact: Option<SyncSender<GraphResult<crate::CompactionReport>>> = None;
    loop {
        // Block for the first item, unless a compact was deferred from the
        // previous turn's commit drain. Channel-closed => owner dropped => exit.
        let work = match pending_compact.take() {
            Some(reply) => Work::Compact { reply },
            None => match receiver.recv() {
                Ok(work) => work,
                Err(_) => return,
            },
        };

        match work {
            Work::Compact { reply } => {
                // Compact is always a batch boundary: it reads + mutates the
                // live graph under the lock, so it cannot share a WAL batch.
                let result = run_protected(|| compact_on_committer(&handles));
                let result = unwrap_protected(result, poisoned);
                let _ = reply.send(result);
            }
            Work::Commit { sealed, reply } => {
                // Drain up to MAX_COMMIT_BATCH commits (BRIEF 1: cap 1).
                let mut batch: Vec<(SealedCommit, SyncSender<GraphResult<CommitOutcome>>)> =
                    vec![(sealed, reply)];
                while batch.len() < MAX_COMMIT_BATCH {
                    match receiver.try_recv() {
                        Ok(Work::Commit { sealed, reply }) => batch.push((sealed, reply)),
                        // A Compact ends the commit batch and is serviced next
                        // turn as a boundary (FIFO preserved).
                        Ok(Work::Compact { reply }) => {
                            pending_compact = Some(reply);
                            break;
                        }
                        Err(_) => break,
                    }
                }
                run_commit_batch(&mut batch, poisoned, &handles);
            }
        }

        if poisoned.load(Ordering::Acquire) {
            return;
        }
    }
}

/// Publish a drained commit batch in FIFO. Each item is published independently
/// (BRIEF 1 keeps `EveryN(1)`, so each `write_commit` fsyncs); a panic in any
/// item poisons the committer and Errs every remaining waiter in the batch.
fn run_commit_batch(
    batch: &mut Vec<(SealedCommit, SyncSender<GraphResult<CommitOutcome>>)>,
    poisoned: &Arc<std::sync::atomic::AtomicBool>,
    handles: &CommitterHandles,
) {
    let mut drained = batch.drain(..);
    for (sealed, reply) in drained.by_ref() {
        if poisoned.load(Ordering::Acquire) {
            // A prior item in this batch panicked: fail every still-unacked
            // waiter so no SyncSender is dropped silently (which would hang its
            // recv() with a RecvError). Never publish after a poison.
            let _ = reply.send(Err(committer_dead()));
            continue;
        }
        let result = run_protected(|| {
            publish_sealed(
                sealed,
                &handles.snapshot,
                &handles.schema_version,
                &handles.providers,
                &handles.durable_providers,
            )
        });
        let result = unwrap_protected(result, poisoned);
        let _ = reply.send(result);
    }
}

/// Run a committer body inside `catch_unwind`. parking_lot does not poison, so
/// a panic leaves locks usable; the engine is poisoned at a higher level
/// instead (no further commits trusted).
fn run_protected<T>(
    body: impl FnOnce() -> GraphResult<T>,
) -> Result<GraphResult<T>, Box<dyn std::any::Any + Send>> {
    std::panic::catch_unwind(AssertUnwindSafe(body))
}

/// Convert a `catch_unwind` result into a `GraphResult`, poisoning the committer
/// on panic so subsequent submits fail fast.
fn unwrap_protected<T>(
    result: Result<GraphResult<T>, Box<dyn std::any::Any + Send>>,
    poisoned: &Arc<std::sync::atomic::AtomicBool>,
) -> GraphResult<T> {
    match result {
        Ok(ok) => ok,
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

/// Compact the live graph in place on the committer thread (the one case where
/// the committer takes the write lock). Mirrors the pre-v1.2 `SharedGraph::compact`
/// body verbatim, but runs on the sole publisher so it serializes with commits.
fn compact_on_committer(handles: &CommitterHandles) -> GraphResult<crate::CompactionReport> {
    let mut guard = handles.shared.write();
    let compacted = crate::compaction::compact_core(&guard)?;
    let dense = Arc::new(compacted.graph);
    *guard = Arc::clone(&dense);
    handles.snapshot.store(dense);
    Ok(compacted.report)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use selene_core::{Change, GraphId, HlcTimestamp, LabelSet, PropertyMap, intern};

    use crate::SharedGraph;
    use crate::durable_provider::DurableProvider;
    use crate::error::GraphError;
    use crate::index_provider::{ProviderError, ProviderTag};

    fn istr(value: &str) -> selene_core::IStr {
        intern(value).expect("interns")
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
            _principal: Option<&[u8]>,
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
        )
        .expect("graph builds with synthetic durable provider")
    }

    #[test]
    fn cancel_cutline_pre_append_aborts_with_no_burned_state() {
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
        // Attach an already-cancelled token after sealing: the committer must
        // abort at the pre-WAL cut-line and never append or publish.
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let sealed = txn.seal(None).unwrap().with_cancel(Arc::clone(&flag));
        let err = shared
            .submit_sealed_for_test(sealed)
            .expect_err("pre-append cancel returns Err");
        assert!(matches!(err, GraphError::Cancelled), "got {err:?}");
        assert_eq!(err.gqlstatus(), "5GQL2");

        // Nothing published, nothing appended.
        assert_eq!(shared.read().node_count(), 0);
        assert_eq!(shared.read().meta.generation, 0);

        // A subsequent uncancelled commit gets WAL seq 1 — the cancelled commit
        // burned no durable sequence (it never appended).
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        let outcome = txn.commit().unwrap();
        assert_eq!(outcome.durable_at, Some(1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_cutline_post_check_is_irrevocable() {
        // When the cut-line samples the token as false it proceeds; flipping the
        // token afterward cannot revoke the published commit.
        let shared = SharedGraph::new(GraphId::new(91_002));
        let mut txn = shared.begin_write();
        let id = txn
            .mutator()
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sealed = txn.seal(None).unwrap().with_cancel(Arc::clone(&flag));
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
            .create_node(LabelSet::single(istr("L")), PropertyMap::new())
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
    fn mid_batch_panic_errs_every_waiter_in_flight() {
        // With MAX_COMMIT_BATCH == 1 each batch holds one commit, so a panic in
        // a batch's only item must Err that waiter. Multiple subsequent waiters
        // queued behind it all fail fast rather than block forever. Drive many
        // panicking commits concurrently and assert every one returns Err in
        // bounded time.
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
}
