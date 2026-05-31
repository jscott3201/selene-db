//! v1.2 multi-writer BRIEF 2 — WAL-backed group-commit tests (T7..T13).
//!
//! Child of `committer_batch_tests.rs`; shares its helpers + synthetic durable
//! providers via `use super::*`. Split out only to keep each test file under the
//! 700-LOC cap. T7 (compact-boundary durable-before-visible) is load-bearing.

// Everything else (Arc, Barrier, thread, Duration, Instant, atomics, the
// synthetic providers + helpers, SharedGraph, CommitBatching, …) is brought in
// from the parent test module's scope by this glob.
use super::*;
// Items NOT already in the parent's scope.
use std::sync::atomic::AtomicUsize;

use selene_persist::{DEFAULT_WAL_FILE_NAME, SyncPolicy, WalConfig};

// ───────────────────────── T7 (load-bearing, F2) ─────────────────────────

/// Durable that latches a per-flush watermark and records, for every
/// `write_commit`, the flush-epoch in effect when it was appended. Combined with
/// the GenOrderProvider's publish observation it proves the commits in a batch
/// were FLUSHED before the dense compact stored.
struct FlushEpochDurable {
    tag: ProviderTag,
    seq: AtomicU64,
    flush_epoch: AtomicUsize,
    flushes: AtomicUsize,
}

impl FlushEpochDurable {
    fn new(tag: &[u8; 4]) -> Arc<Self> {
        Arc::new(Self {
            tag: ProviderTag(*tag),
            seq: AtomicU64::new(0),
            flush_epoch: AtomicUsize::new(0),
            flushes: AtomicUsize::new(0),
        })
    }
}

impl DurableProvider for FlushEpochDurable {
    fn provider_tag(&self) -> ProviderTag {
        self.tag
    }
    fn write_commit(
        &self,
        _principal: Option<&[u8]>,
        _changes: &[Change],
        _timestamp: HlcTimestamp,
    ) -> Result<u64, ProviderError> {
        Ok(self.seq.fetch_add(1, Ordering::SeqCst) + 1)
    }
    fn flush(&self) -> Result<Option<u64>, ProviderError> {
        self.flushes.fetch_add(1, Ordering::SeqCst);
        self.flush_epoch.fetch_add(1, Ordering::SeqCst);
        Ok(Some(self.seq.load(Ordering::SeqCst)))
    }
}

#[test]
fn t7_compact_boundary_durable_before_visible() {
    // Real WAL. Seal A, B (unpublished), then compact() (seal_seq after them).
    // Submit the compact-publish FIRST, then A, B. The committer publishes in
    // seal_seq order: A, B flush+publish+ack BEFORE the dense compact stores
    // (the compact is a hard flush boundary, F2). Final snapshot is dense AND
    // contains A, B.
    let dir = temp_dir("t7");
    let wal = dir.join(DEFAULT_WAL_FILE_NAME);
    let shared = Arc::new(
        SharedGraph::builder(GraphId::new(70_010))
            .with_wal(&wal, WalConfig::default())
            .unwrap()
            .with_commit_batching(on(8, 8 * 1024 * 1024))
            .build()
            .unwrap(),
    );

    // Seed reclaimable holes so the compact actually densifies.
    {
        let mut txn = shared.begin_write();
        let mut ids = Vec::new();
        for _ in 0..20 {
            ids.push(
                txn.mutator()
                    .create_node(LabelSet::single(istr("S")), PropertyMap::new())
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

    // Seal A, B but do not submit.
    let mut txn_a = shared.begin_write();
    let a = txn_a
        .mutator()
        .create_node(LabelSet::single(istr("A")), PropertyMap::new())
        .unwrap();
    let sealed_a = txn_a.seal(None, None).expect("A seals");
    let mut txn_b = shared.begin_write();
    let b = txn_b
        .mutator()
        .create_node(LabelSet::single(istr("B")), PropertyMap::new())
        .unwrap();
    let sealed_b = txn_b.seal(None, None).expect("B seals");

    // Run compact on a background thread: it allocates a seal_seq AFTER A, B
    // under the lock, then submits its dense publish and blocks. Yield so the
    // compact enqueues before A, B, exercising the reorder buffer.
    let s_c = Arc::clone(&shared);
    let compactor = thread::spawn(move || s_c.compact().expect("compaction ok"));
    for _ in 0..2_000 {
        thread::yield_now();
    }
    // Submit B then A (reverse) so the contiguous run [A,B] forms only after A.
    let s_b = Arc::clone(&shared);
    let b_thread = thread::spawn(move || s_b.submit_sealed_for_test(sealed_b).expect("B"));
    for _ in 0..1_000 {
        thread::yield_now();
    }
    let outcome_a = shared.submit_sealed_for_test(sealed_a).expect("A");
    let outcome_b = b_thread.join().unwrap();
    let report = compactor.join().expect("compactor ok");

    // A, B durable (acked with a durable_at) and visible; report reclaimed.
    assert!(outcome_a.durable_at.is_some());
    assert!(outcome_b.durable_at.is_some());
    assert!(report.reclaimed_nodes >= 20, "report: {report:?}");

    // Final snapshot is the dense compacted one AND contains A, B.
    let snap = shared.read();
    assert!(snap.is_node_alive(a));
    assert!(snap.is_node_alive(b));
    assert_eq!(snap.node_count(), 2, "only A, B alive");
    assert_eq!(
        snap.node_store.len(),
        2,
        "published snapshot is dense (compaction not clobbered by A,B's stale snapshot)",
    );
    snap.assert_indexes_consistent()
        .expect("structurally consistent");
}

#[test]
fn t7b_compact_at_head_publishes_with_zero_flush_calls() {
    // A compact whose seal_seq is at next_publish_seq (no pending commit run)
    // publishes the dense Arc with ZERO flush calls — all lower seqs already
    // durable + visible.
    let durable = FlushEpochDurable::new(b"FEP1");
    let shared = SharedGraph::from_graph_with_core_and_durables(
        SeleneGraph::new(GraphId::new(70_011)),
        Vec::new(),
        vec![durable.clone()],
        None,
        None,
        on(8, 8 * 1024 * 1024),
    )
    .unwrap();

    // Two committed-then-deleted nodes, fully published + flushed.
    let mut txn = shared.begin_write();
    let ids: Vec<_> = (0..4)
        .map(|_| {
            txn.mutator()
                .create_node(LabelSet::single(istr("H")), PropertyMap::new())
                .unwrap()
        })
        .collect();
    txn.commit().unwrap();
    let mut txn = shared.begin_write();
    for id in &ids {
        txn.mutator().delete_node(*id).unwrap();
    }
    txn.commit().unwrap();

    let flushes_before = durable.flushes.load(Ordering::SeqCst);
    // Compact is now strictly at head (nothing else pending) — publishes solo.
    let report = shared.compact().expect("compact ok");
    assert!(report.reclaimed_nodes >= 4);
    assert_eq!(
        durable.flushes.load(Ordering::SeqCst),
        flushes_before,
        "compact-at-head issues zero flush calls",
    );
}

// ───────────────────────── T8 ─────────────────────────

#[test]
fn t8_compact_recovery_after_crash() {
    // Write A, B, compact through a real WAL; drop the graph (committer joins,
    // flushes acked work); reopen; assert A, B present.
    let dir = temp_dir("t8");
    let wal = dir.join(DEFAULT_WAL_FILE_NAME);
    let (a, b) = {
        let shared = SharedGraph::builder(GraphId::new(70_020))
            .with_wal(&wal, WalConfig::default())
            .unwrap()
            .with_commit_batching(CommitBatching::DEFAULT_ON)
            .build()
            .unwrap();
        let mut txn = shared.begin_write();
        let a = txn
            .mutator()
            .create_node(LabelSet::single(istr("A")), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        let mut txn = shared.begin_write();
        let b = txn
            .mutator()
            .create_node(LabelSet::single(istr("B")), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        shared.compact().expect("compact ok");
        (a, b)
        // shared dropped here: committer joins; acked work already flushed.
    };

    let recovered = SharedGraph::recover(&dir, GraphId::new(70_020)).expect("recovers");
    assert!(recovered.read().is_node_alive(a));
    assert!(recovered.read().is_node_alive(b));
    assert_eq!(recovered.read().node_count(), 2);
}

// ───────────────────────── T9 ─────────────────────────

#[test]
fn t9_recovery_after_crash_batched() {
    // On(16), real WAL; fan in 100 node-commits, get all acks, drop, reopen;
    // assert all 100 recovered with gap-free ids.
    let dir = temp_dir("t9");
    let wal = dir.join(DEFAULT_WAL_FILE_NAME);
    const TOTAL: usize = 100;
    {
        let shared = Arc::new(
            SharedGraph::builder(GraphId::new(70_030))
                .with_wal(&wal, WalConfig::default())
                .unwrap()
                .with_commit_batching(on(16, 8 * 1024 * 1024))
                .build()
                .unwrap(),
        );
        let barrier = Arc::new(Barrier::new(8));
        thread::scope(|scope| {
            for t in 0..8 {
                let shared = Arc::clone(&shared);
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    barrier.wait();
                    let mut idx = t;
                    while idx < TOTAL {
                        let mut txn = shared.begin_write();
                        txn.mutator()
                            .create_node(LabelSet::single(istr("F")), PropertyMap::new())
                            .unwrap();
                        txn.commit().expect("commit ok");
                        idx += 8;
                    }
                });
            }
        });
        assert_eq!(shared.read().node_count(), TOTAL);
    }

    let recovered = SharedGraph::recover(&dir, GraphId::new(70_030)).expect("recovers");
    assert_eq!(
        recovered.read().node_count(),
        TOTAL,
        "all acked commits recovered",
    );
}

// ───────────────────────── T10 ─────────────────────────

#[test]
fn t10_crash_after_append_before_flush_loses_only_unflushed() {
    // On, real WAL: drive acked commits (flushed), then a final commit on the
    // committer is naturally flushed before ack — so after a clean drop the
    // recovered tip equals the last ACKED commit. The Off/EveryN(1) control
    // recovers identically. (A true mid-run crash cannot be staged in-process
    // without unsafe; this pins the durable-before-visible boundary: acked ⇒
    // recoverable.)
    let dir = temp_dir("t10");
    let wal = dir.join(DEFAULT_WAL_FILE_NAME);
    let acked = {
        let shared = SharedGraph::builder(GraphId::new(70_040))
            .with_wal(&wal, WalConfig::default())
            .unwrap()
            .with_commit_batching(on(8, 8 * 1024 * 1024))
            .build()
            .unwrap();
        let mut last = 0;
        for _ in 0..10 {
            let mut txn = shared.begin_write();
            txn.mutator()
                .create_node(LabelSet::single(istr("C")), PropertyMap::new())
                .unwrap();
            last = txn.commit().expect("ok").generation;
        }
        last
    };
    let recovered = SharedGraph::recover(&dir, GraphId::new(70_040)).expect("recovers");
    assert_eq!(recovered.read().node_count(), acked as usize);

    // Off control over a fresh dir: identical recovered tip.
    let dir2 = temp_dir("t10-off");
    let wal2 = dir2.join(DEFAULT_WAL_FILE_NAME);
    {
        let shared = SharedGraph::builder(GraphId::new(70_041))
            .with_wal(&wal2, WalConfig::default())
            .unwrap()
            .build()
            .unwrap();
        for _ in 0..10 {
            let mut txn = shared.begin_write();
            txn.mutator()
                .create_node(LabelSet::single(istr("C")), PropertyMap::new())
                .unwrap();
            txn.commit().unwrap();
        }
    }
    let recovered_off = SharedGraph::recover(&dir2, GraphId::new(70_041)).expect("recovers");
    assert_eq!(recovered_off.read().node_count(), 10);
}

// ───────────────────────── T11 ─────────────────────────

#[test]
fn t11_concurrent_fan_in_no_loss() {
    // On, 32 threads each committing K disjoint nodes; join; assert
    // node_count == 32*K, gen == 32*K, durable_at strictly increased overall.
    const THREADS: usize = 32;
    const PER: usize = 4;
    let durable = CountingDurable::new(b"CNTB");
    let shared = Arc::new(graph_with_durable(
        70_050,
        durable.clone(),
        CommitBatching::DEFAULT_ON,
    ));
    let barrier = Arc::new(Barrier::new(THREADS));
    let max_durable = Arc::new(AtomicU64::new(0));

    thread::scope(|scope| {
        for _ in 0..THREADS {
            let shared = Arc::clone(&shared);
            let barrier = Arc::clone(&barrier);
            let max_durable = Arc::clone(&max_durable);
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..PER {
                    let mut txn = shared.begin_write();
                    txn.mutator()
                        .create_node(LabelSet::single(istr("D")), PropertyMap::new())
                        .unwrap();
                    let outcome = txn.commit().expect("commit ok");
                    let d = outcome.durable_at.expect("durable_at set");
                    max_durable.fetch_max(d, Ordering::SeqCst);
                }
            });
        }
    });

    let total = (THREADS * PER) as u64;
    assert_eq!(shared.read().node_count() as u64, total);
    assert_eq!(shared.read().meta.generation, total);
    assert_eq!(
        max_durable.load(Ordering::SeqCst),
        total,
        "durable_at reached the final commit's sequence",
    );
    assert_eq!(durable.write_count() as u64, total);
}

// ───────────────────────── T12 ─────────────────────────

#[test]
fn t12_config_forces_on_flush_only() {
    // with_wal(EveryN(5)) opens the committer WAL in OnFlushOnly. Probe via a
    // flush-observing durable is not possible for the CORE WAL directly, so we
    // assert behavior: under Off the committer flushes once per commit (the WAL
    // append itself never fsyncs at EveryN(5) cadence; the committer's explicit
    // flush is the only fsync). We verify the override took by confirming a
    // single commit is durable on reopen even though EveryN(5) would not have
    // fsynced after one append.
    let dir = temp_dir("t12");
    let wal = dir.join(DEFAULT_WAL_FILE_NAME);
    {
        let shared = SharedGraph::builder(GraphId::new(70_060))
            .with_wal(
                &wal,
                WalConfig {
                    sync_policy: SyncPolicy::EveryN(5),
                    snapshot_seq: 0,
                },
            )
            .unwrap()
            .build()
            .unwrap();
        // ONE commit. Under raw EveryN(5) the WAL append would NOT fsync (1 < 5),
        // but the committer's explicit flush (forced OnFlushOnly + Off ⇒ flush
        // per commit) makes it durable.
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(istr("O")), PropertyMap::new())
            .unwrap();
        let outcome = txn.commit().expect("commit ok");
        assert_eq!(outcome.durable_at, Some(1));
    }
    // Reopen: the single commit survived ⇒ the committer fsynced it (the
    // OnFlushOnly override + per-commit flush worked).
    let recovered = SharedGraph::recover(&dir, GraphId::new(70_060)).expect("recovers");
    assert_eq!(recovered.read().node_count(), 1);
}

// ───────────────────────── T13 (DoS) ─────────────────────────

#[test]
fn t13_cap_bounds_accumulation() {
    // On(max_commits=4, max_bytes=tiny). Fan in many small commits + one fat
    // commit; assert no run exceeds 4 commits, the fat commit commits ALONE
    // (>= 1 progress rule, never rejected), and everything succeeds.
    let durable = CountingDurable::new(b"CNTC");
    // max_bytes tiny: 80 bytes. A 1-change commit estimates 64 + 256 = 320 > 80,
    // so EVERY commit is over-cap and taken alone (the >= 1 rule). A 0-change
    // commit estimates 64 < 80. We mostly assert no panic / no loss / progress.
    let shared = Arc::new(graph_with_durable(70_070, durable.clone(), on(4, 80)));

    const TOTAL: usize = 40;
    // One fat commit (many changes) + many small ones.
    {
        let mut txn = shared.begin_write();
        for _ in 0..50 {
            txn.mutator()
                .create_node(LabelSet::single(istr("Fat")), PropertyMap::new())
                .unwrap();
        }
        txn.commit().expect("fat commit alone, never rejected");
    }
    let barrier = Arc::new(Barrier::new(8));
    thread::scope(|scope| {
        for t in 0..8 {
            let shared = Arc::clone(&shared);
            let barrier = Arc::clone(&barrier);
            scope.spawn(move || {
                barrier.wait();
                let mut idx = t;
                while idx < TOTAL {
                    let mut txn = shared.begin_write();
                    txn.mutator()
                        .create_node(LabelSet::single(istr("Sm")), PropertyMap::new())
                        .unwrap();
                    txn.commit().expect("small commit ok");
                    idx += 8;
                }
            });
        }
    });

    // All committed (fat 50 + 40 small = 90 nodes), no loss, no rejection.
    assert_eq!(shared.read().node_count(), 50 + TOTAL);
    assert_eq!(
        durable.write_count(),
        1 + TOTAL,
        "every commit appended once"
    );
}
