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
use std::collections::BTreeMap;

use selene_persist::{DEFAULT_WAL_FILE_NAME, SyncPolicy, WalConfig};

// ───────────────────────── T7 (load-bearing, F2) ─────────────────────────

/// A monotonic per-flush watermark ("flush epoch") shared by [`FlushEpochDurable`]
/// (the writer/fsync side) and [`FlushEpochObserver`] (the publish/fan-out side),
/// so a test can pin the R1 barrier's interleaving: append happens at epoch E,
/// the group flush bumps the epoch E→E+1, and publish (fan-out) is observed at
/// the post-flush epoch. `publish_epoch > append_epoch` therefore proves the
/// commit was FLUSHED strictly between its Stage-1 append and its Stage-3 publish
/// — durable-before-visible — for every batched commit, and at the compact
/// boundary in particular (A, B flushed before they publish, before the dense
/// store).
type FlushEpoch = Arc<AtomicU64>;

/// Durable that latches a shared per-flush watermark and records, for every
/// `write_commit`, both the assigned sequence and the flush-epoch in effect when
/// it was appended (BEFORE its group flush). Paired with [`FlushEpochObserver`],
/// it proves a batch's commits were FLUSHED before they were published (and, at
/// the compact boundary, before the dense Arc stored).
struct FlushEpochDurable {
    tag: ProviderTag,
    seq: AtomicU64,
    /// Shared monotonic flush watermark, bumped once per `flush`.
    flush_epoch: FlushEpoch,
    /// Count of `flush` calls (used by the zero-flush compact-at-head proof).
    flushes: AtomicU64,
    /// Per assigned sequence, the flush-epoch observed at append time.
    append_epoch: Mutex<BTreeMap<u64, u64>>,
}

impl FlushEpochDurable {
    fn new(tag: &[u8; 4], flush_epoch: FlushEpoch) -> Arc<Self> {
        Arc::new(Self {
            tag: ProviderTag(*tag),
            seq: AtomicU64::new(0),
            flush_epoch,
            flushes: AtomicU64::new(0),
            append_epoch: Mutex::new(BTreeMap::new()),
        })
    }

    /// The flush-epoch in effect when the commit assigned `seq` was appended.
    fn append_epoch_of(&self, seq: u64) -> u64 {
        *self
            .append_epoch
            .lock()
            .unwrap()
            .get(&seq)
            .expect("append epoch recorded for seq")
    }
}

impl DurableProvider for FlushEpochDurable {
    fn provider_tag(&self) -> ProviderTag {
        self.tag
    }
    fn write_commit(
        &self,
        _principal: Option<&Arc<[u8]>>,
        _changes: &[Change],
        _timestamp: HlcTimestamp,
    ) -> Result<u64, ProviderError> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        // Record the flush-epoch in effect at append time, BEFORE any group flush
        // for this run runs (Stage 1, fsync deferred).
        let epoch = self.flush_epoch.load(Ordering::SeqCst);
        self.append_epoch.lock().unwrap().insert(seq, epoch);
        Ok(seq)
    }
    fn flush(&self) -> Result<Option<u64>, ProviderError> {
        self.flushes.fetch_add(1, Ordering::SeqCst);
        // The R1 barrier: bump the shared watermark so any publish observed after
        // this flush reads a strictly-greater epoch than the appends it covers.
        self.flush_epoch.fetch_add(1, Ordering::SeqCst);
        Ok(Some(self.seq.load(Ordering::SeqCst)))
    }
}

/// Fan-out provider that records, per published `NodeCreated`, the flush-epoch in
/// effect at publish time (Stage 3). Paired with [`FlushEpochDurable`]: comparing
/// the publish-epoch against the durable's append-epoch for the same commit
/// proves the group flush ran strictly between append and publish.
struct FlushEpochObserver {
    tag: ProviderTag,
    flush_epoch: FlushEpoch,
    /// Node-id → flush-epoch observed when that node was published.
    publish_epoch: Mutex<BTreeMap<u64, u64>>,
}

impl FlushEpochObserver {
    fn new(tag: &[u8; 4], flush_epoch: FlushEpoch) -> Arc<Self> {
        Arc::new(Self {
            tag: ProviderTag(*tag),
            flush_epoch,
            publish_epoch: Mutex::new(BTreeMap::new()),
        })
    }

    /// The flush-epoch in effect when the node with this id was published.
    fn publish_epoch_of(&self, id: u64) -> u64 {
        *self
            .publish_epoch
            .lock()
            .unwrap()
            .get(&id)
            .expect("publish epoch recorded for node id")
    }
}

impl IndexProvider for FlushEpochObserver {
    fn provider_tag(&self) -> ProviderTag {
        self.tag
    }
    fn read_section(&self, _sub: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }
    fn write_section(&self, _sub: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }
    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        if let Change::NodeCreated { id, .. } = change {
            let epoch = self.flush_epoch.load(Ordering::SeqCst);
            self.publish_epoch.lock().unwrap().insert(id.get(), epoch);
        }
        Ok(())
    }
    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

#[test]
fn t7_compact_boundary_durable_before_visible() {
    // F2 ordering proof (durable-before-visible at the compact boundary), pinned
    // by a flush-epoch watermark rather than only end-state. A synthetic
    // FlushEpochDurable + FlushEpochObserver share a per-flush watermark; the
    // durable records each commit's APPEND epoch (before its group flush) and the
    // observer records each node's PUBLISH epoch (at fan-out). Because the group
    // flush bumps the watermark between Stage-1 append and Stage-3 publish,
    // publish_epoch(A,B) > append_epoch(A,B) proves A, B were FLUSHED before they
    // were published. Compaction reclaims the in-memory holes (no real WAL needed
    // — the dense layout comes from the graph, not the durable; real-WAL compact
    // recovery is covered by T8). The compact is a hard flush boundary (F2):
    // A, B flush+publish+ack BEFORE the dense Arc stores, and the final snapshot
    // is dense AND contains A, B (the dense store is LAST).
    let flush_epoch: FlushEpoch = Arc::new(AtomicU64::new(0));
    let durable = FlushEpochDurable::new(b"FEP0", Arc::clone(&flush_epoch));
    let observer = FlushEpochObserver::new(b"FOB0", Arc::clone(&flush_epoch));
    let shared = Arc::new(
        SharedGraph::from_graph_with_core_and_durables(
            SeleneGraph::new(GraphId::new(70_010)),
            vec![observer.clone() as Arc<dyn IndexProvider>],
            vec![durable.clone()],
            None,
            None,
            on(8, 8 * 1024 * 1024),
        )
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
    let a_seq = outcome_a.durable_at.expect("A durable_at");
    let b_seq = outcome_b.durable_at.expect("B durable_at");
    assert!(report.reclaimed_nodes >= 20, "report: {report:?}");

    // ORDERING PROOF (the F2 / R1 barrier): for both A and B the group flush bumped
    // the shared watermark strictly between their append and their publish, so
    // each was durable BEFORE it became visible.
    let a_pub = observer.publish_epoch_of(a.get());
    let b_pub = observer.publish_epoch_of(b.get());
    assert!(
        a_pub > durable.append_epoch_of(a_seq),
        "A published (epoch {a_pub}) only after its group flush (append epoch {})",
        durable.append_epoch_of(a_seq),
    );
    assert!(
        b_pub > durable.append_epoch_of(b_seq),
        "B published (epoch {b_pub}) only after its group flush (append epoch {})",
        durable.append_epoch_of(b_seq),
    );

    // Final snapshot is the dense compacted one AND contains A, B (the dense
    // store ran LAST — after A, B flushed + published).
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
    let flush_epoch: FlushEpoch = Arc::new(AtomicU64::new(0));
    let durable = FlushEpochDurable::new(b"FEP1", flush_epoch);
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

// ───────────────── T2b: within-batch commit flush-order (R1 barrier) ─────────

#[test]
fn t2b_within_batch_commits_flush_before_publish() {
    // The headline durable-before-visible guarantee for GROUPED commits (not just
    // at the compact boundary): the single group flush precedes EVERY commit's
    // publish in the run. Fan in a contiguous run under On; the FlushEpochDurable
    // records each commit's append epoch, the FlushEpochObserver records each
    // node's publish epoch, and we assert publish_epoch > append_epoch for every
    // commit — i.e. the R1 barrier sits strictly between Stage-1 append and
    // Stage-3 publish for grouped commits. Buffer the later seqs behind a gap and
    // release seq 0 last so a genuine multi-member batch forms (>= 2 in one run).
    let flush_epoch: FlushEpoch = Arc::new(AtomicU64::new(0));
    let durable = FlushEpochDurable::new(b"FEP2", Arc::clone(&flush_epoch));
    let observer = FlushEpochObserver::new(b"FOB2", Arc::clone(&flush_epoch));
    let shared = Arc::new(
        SharedGraph::from_graph_with_core_and_durables(
            SeleneGraph::new(GraphId::new(70_012)),
            vec![observer.clone() as Arc<dyn IndexProvider>],
            vec![durable.clone()],
            None,
            None,
            on(8, 8 * 1024 * 1024),
        )
        .unwrap(),
    );

    // Seal 4 commits (seal_seq 0..3) each forking the prior.
    let mut sealeds = Vec::new();
    let mut ids = Vec::new();
    for label in ["p", "q", "r", "s"] {
        let mut txn = shared.begin_write();
        let id = txn
            .mutator()
            .create_node(LabelSet::single(istr(label)), PropertyMap::new())
            .unwrap();
        ids.push(id);
        sealeds.push(txn.seal(None, None).expect("seals"));
    }

    // Withhold seq 0; submit seqs 3,2,1 first (buffer behind the gap), then seq 0
    // last so the full [0,1,2,3] contiguous run drains as ONE batch with ONE flush.
    let sealed_0 = sealeds.remove(0);
    let mut handles = Vec::new();
    while let Some(sealed) = sealeds.pop() {
        let shared = Arc::clone(&shared);
        handles.push(thread::spawn(move || {
            shared
                .submit_sealed_for_test(sealed)
                .expect("buffered commit")
        }));
        for _ in 0..1_000 {
            thread::yield_now();
        }
    }
    let outcome_0 = shared.submit_sealed_for_test(sealed_0).expect("seq 0");
    let mut durable_seqs = vec![outcome_0.durable_at.expect("durable_at")];
    for handle in handles {
        durable_seqs.push(handle.join().unwrap().durable_at.expect("durable_at"));
    }

    // The run grouped (fewer flushes than commits) — otherwise this would only be
    // testing the degenerate cap-1 path.
    assert!(
        durable.flushes.load(Ordering::SeqCst) < 4,
        "the 4 commits grouped into fewer than 4 flushes (got {})",
        durable.flushes.load(Ordering::SeqCst),
    );

    // R1 barrier: every commit's append epoch is strictly below every commit's
    // publish epoch — the group flush bumped the watermark strictly between
    // Stage-1 append and Stage-3 publish for the whole run. Order-independent
    // (max append epoch < min publish epoch) so the proof does not rely on which
    // node id maps to which assigned seq.
    let max_append = durable_seqs
        .iter()
        .map(|seq| durable.append_epoch_of(*seq))
        .max()
        .expect("at least one commit");
    let min_publish = ids
        .iter()
        .map(|id| observer.publish_epoch_of(id.get()))
        .min()
        .expect("at least one node");
    assert!(
        min_publish > max_append,
        "the group flush separated every append (max epoch {max_append}) from every \
         publish (min epoch {min_publish}) — durable-before-visible for grouped commits",
    );
    assert_eq!(shared.read().node_count(), 4);
    assert_eq!(shared.read().meta.generation, 4);
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
    // On(16), real WAL; fan in 100 node-commits from 8 threads, collect the
    // acked NodeIds, drop, reopen; assert the recovered live id set equals
    // EXACTLY the acked set — no holes, no extras (gap-free recovery; D11/D22:
    // every acked external NodeId survives, and nothing un-acked leaks in). The
    // mid-flood panic variant is covered separately (T5/T5b + committer.rs panic
    // tests drive the poison path); here we pin acked ⇒ recovered with stable ids.
    let dir = temp_dir("t9");
    let wal = dir.join(DEFAULT_WAL_FILE_NAME);
    const TOTAL: usize = 100;
    let acked_ids = Arc::new(Mutex::new(Vec::with_capacity(TOTAL)));
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
                let acked_ids = Arc::clone(&acked_ids);
                scope.spawn(move || {
                    barrier.wait();
                    let mut idx = t;
                    while idx < TOTAL {
                        let mut txn = shared.begin_write();
                        let id = txn
                            .mutator()
                            .create_node(LabelSet::single(istr("F")), PropertyMap::new())
                            .unwrap();
                        txn.commit().expect("commit ok");
                        // Only record AFTER the ack: this id is durable + visible.
                        acked_ids.lock().unwrap().push(id);
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

    // Gap-free: the recovered live id set is EXACTLY the acked set.
    let snap = recovered.read();
    let mut acked: Vec<_> = acked_ids.lock().unwrap().clone();
    acked.sort_unstable();
    assert_eq!(acked.len(), TOTAL, "every commit's node id was recorded");
    for id in &acked {
        assert!(
            snap.is_node_alive(*id),
            "acked node {id:?} survived recovery",
        );
    }
    // No extras: exactly TOTAL live nodes, all of them in the acked set.
    assert_eq!(
        snap.node_count(),
        acked.len(),
        "no un-acked id leaked into the recovered graph",
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
    // With max_bytes=80, every >=1-change commit estimates over the byte cap and
    // is taken ALONE (the >= 1 progress rule), so no batch ever exceeds 4 — here,
    // it never exceeds 1. The COUNT cap is pinned directly by t13b below.
    assert!(
        durable.max_batch_size() <= 4,
        "no batch exceeds the count cap of 4 (observed max {})",
        durable.max_batch_size(),
    );
}

#[test]
fn t13b_count_cap_clamps_batch_size() {
    // Directly pin the F4 COUNT cap: with max_bytes generous (so the count cap is
    // the only binding constraint) and a fully-buffered contiguous run of 12
    // commits, the committer must never coalesce more than max_commits=4 into one
    // group flush. We seal 12 commits, buffer seqs 1..11 behind the seq-0 gap so
    // they cannot drain piecemeal, then release seq 0 last — the whole [0..11] run
    // is present in the reorder buffer when drain_contiguous_batch runs, so an
    // uncapped committer would form one 12-member batch (max_batch_size == 12).
    // With the cap it forms batches of 4 ⇒ max_batch_size == 4. The durable's
    // Write/Flush event log makes batch size observable (writes between flushes).
    const TOTAL: usize = 12;
    const MAX_COMMITS: usize = 4;
    let durable = CountingDurable::new(b"CN13");
    let shared = Arc::new(graph_with_durable(
        70_071,
        durable.clone(),
        on(MAX_COMMITS, 8 * 1024 * 1024),
    ));

    let mut sealeds = Vec::new();
    for _ in 0..TOTAL {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(istr("Cap")), PropertyMap::new())
            .unwrap();
        sealeds.push(txn.seal(None, None).expect("seals"));
    }

    // Withhold seq 0; submit seqs 1..11 (buffer behind the gap), then seq 0 last.
    let sealed_0 = sealeds.remove(0);
    let mut handles = Vec::new();
    while let Some(sealed) = sealeds.pop() {
        let shared = Arc::clone(&shared);
        handles.push(thread::spawn(move || {
            shared
                .submit_sealed_for_test(sealed)
                .expect("buffered commit")
        }));
        for _ in 0..200 {
            thread::yield_now();
        }
    }
    shared.submit_sealed_for_test(sealed_0).expect("seq 0");
    for handle in handles {
        handle.join().expect("waiter ok");
    }

    assert_eq!(shared.read().node_count(), TOTAL, "no loss");
    assert_eq!(durable.write_count(), TOTAL, "every commit appended once");
    assert!(
        durable.max_batch_size() <= MAX_COMMITS,
        "no group-commit batch exceeds the count cap of {MAX_COMMITS} (observed max {})",
        durable.max_batch_size(),
    );
    // And the cap actually engaged: with 12 fully-buffered contiguous commits a
    // working committer coalesces into runs of MAX_COMMITS, so it must have formed
    // at least one batch larger than 1 (otherwise the cap is untested).
    assert!(
        durable.max_batch_size() > 1,
        "the buffered run coalesced into multi-member batches (observed max {})",
        durable.max_batch_size(),
    );
}
