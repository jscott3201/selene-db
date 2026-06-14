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
