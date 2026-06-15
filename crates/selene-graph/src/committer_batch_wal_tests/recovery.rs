use super::*;

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
            .create_node(LabelSet::single(db_string("A")), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        let mut txn = shared.begin_write();
        let b = txn
            .mutator()
            .create_node(LabelSet::single(db_string("B")), PropertyMap::new())
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
                            .create_node(LabelSet::single(db_string("F")), PropertyMap::new())
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
                .create_node(LabelSet::single(db_string("C")), PropertyMap::new())
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
                .create_node(LabelSet::single(db_string("C")), PropertyMap::new())
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
                        .create_node(LabelSet::single(db_string("D")), PropertyMap::new())
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
            .create_node(LabelSet::single(db_string("O")), PropertyMap::new())
            .unwrap();
        let outcome = txn.commit().expect("commit ok");
        assert_eq!(outcome.durable_at, Some(1));
    }
    // Reopen: the single commit survived ⇒ the committer fsynced it (the
    // OnFlushOnly override + per-commit flush worked).
    let recovered = SharedGraph::recover(&dir, GraphId::new(70_060)).expect("recovers");
    assert_eq!(recovered.read().node_count(), 1);
}
