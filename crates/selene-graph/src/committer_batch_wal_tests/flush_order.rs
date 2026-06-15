use super::*;

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

    // Seal A, B but do not submit.
    let mut txn_a = shared.begin_write();
    let a = txn_a
        .mutator()
        .create_node(LabelSet::single(db_string("A")), PropertyMap::new())
        .unwrap();
    let sealed_a = txn_a.seal(None, None).expect("A seals");
    let mut txn_b = shared.begin_write();
    let b = txn_b
        .mutator()
        .create_node(LabelSet::single(db_string("B")), PropertyMap::new())
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
                .create_node(LabelSet::single(db_string("H")), PropertyMap::new())
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
            .create_node(LabelSet::single(db_string(label)), PropertyMap::new())
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
