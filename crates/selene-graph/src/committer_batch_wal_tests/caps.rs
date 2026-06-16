use super::*;

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
                .create_node(LabelSet::single(db_string("Fat")), PropertyMap::new())
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
                        .create_node(LabelSet::single(db_string("Sm")), PropertyMap::new())
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
            .create_node(LabelSet::single(db_string("Cap")), PropertyMap::new())
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
