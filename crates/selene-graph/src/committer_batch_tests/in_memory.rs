use super::*;

// ───────────────────────── T1 (load-bearing) ─────────────────────────

#[test]
fn t1_off_equals_brief1_fsync_count() {
    // OFF == BRIEF 1 PIN: a counting durable records every write_commit + flush.
    // Under Off, each commit is exactly one write_commit followed by one flush
    // (one fsync per commit), in that order, with durable_at advancing +1 per
    // commit. A schema DDL and a compact ride the same path.
    let durable = CountingDurable::new(b"CNT1");
    let shared = graph_with_durable(70_001, durable.clone(), CommitBatching::Off);

    // Three plain commits.
    for idx in 0..3 {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(db_string("L")), PropertyMap::new())
            .unwrap();
        let outcome = txn.commit().expect("commit ok");
        assert_eq!(outcome.generation, idx + 1);
        // durable_at comes from the counting provider's returned sequence.
        assert_eq!(outcome.durable_at, Some(idx + 1));
    }

    // A schema DDL (property index create) — also a Work::Commit through the
    // same one-append-one-flush path.
    shared
        .create_property_index(db_string("L"), db_string("k"), crate::TypedIndexKind::I64)
        .expect("index create ok");

    // After 3 commits + 1 DDL: 4 writes, 4 flushes, strictly interleaved
    // write,flush,write,flush,...
    let events = durable.events();
    assert_eq!(durable.write_count(), 4, "events: {events:?}");
    assert_eq!(
        durable.flush_count(),
        durable.write_count(),
        "OFF: exactly one flush per commit (events: {events:?})",
    );
    for pair in events.chunks(2) {
        assert!(
            matches!(pair, [DurableEvent::Write(_), DurableEvent::Flush(_)]),
            "OFF interleaves write then flush per commit, got {pair:?}",
        );
    }

    // A compact publishes solo with ZERO additional flush calls (all lower seqs
    // already durable + visible) and adds no write.
    let writes_before = durable.write_count();
    let flushes_before = durable.flush_count();
    shared.compact().expect("compact ok");
    assert_eq!(
        durable.write_count(),
        writes_before,
        "compact appends nothing",
    );
    assert_eq!(
        durable.flush_count(),
        flushes_before,
        "compact issues zero flush calls",
    );
}

#[test]
fn t1b_reverse_order_seal_pair_off_one_flush_each() {
    // Two commits sealed A then B, submitted in reverse, still publish in
    // seal_seq order under Off — and each gets exactly one flush.
    let durable = CountingDurable::new(b"CNT2");
    let shared = Arc::new(graph_with_durable(
        70_002,
        durable.clone(),
        CommitBatching::Off,
    ));

    let mut txn_a = shared.begin_write();
    txn_a
        .mutator()
        .create_node(LabelSet::single(db_string("A")), PropertyMap::new())
        .unwrap();
    let sealed_a = txn_a.seal(None, None).expect("A seals");

    let mut txn_b = shared.begin_write();
    txn_b
        .mutator()
        .create_node(LabelSet::single(db_string("B")), PropertyMap::new())
        .unwrap();
    let sealed_b = txn_b.seal(None, None).expect("B seals");

    let shared_b = Arc::clone(&shared);
    let b_thread = thread::spawn(move || {
        shared_b
            .submit_sealed_for_test(sealed_b)
            .expect("B publishes")
    });
    for _ in 0..1_000 {
        thread::yield_now();
    }
    let outcome_a = shared
        .submit_sealed_for_test(sealed_a)
        .expect("A publishes");
    let outcome_b = b_thread.join().expect("B thread ok");

    assert_eq!(outcome_a.generation, 1);
    assert_eq!(outcome_b.generation, 2);
    // Two commits ⇒ two writes + two flushes (one flush per commit under Off).
    assert_eq!(durable.write_count(), 2);
    assert_eq!(durable.flush_count(), 2);
    assert_eq!(shared.read().node_count(), 2);
}

// ───────────────────────── T2 ─────────────────────────

#[test]
fn t2_on_path_groups_fsyncs() {
    // Fan in M commits from K threads under DEFAULT_ON; the committer coalesces
    // contiguous runs, so flush_count << write_count, every reply Ok with a
    // monotonic durable_at, and the final node count matches.
    const THREADS: usize = 8;
    const PER_THREAD: usize = 64;
    let durable = CountingDurable::new(b"CNT3");
    let shared = Arc::new(graph_with_durable(
        70_003,
        durable.clone(),
        CommitBatching::DEFAULT_ON,
    ));
    let barrier = Arc::new(Barrier::new(THREADS));

    thread::scope(|scope| {
        for thread_idx in 0..THREADS {
            let shared = Arc::clone(&shared);
            let barrier = Arc::clone(&barrier);
            scope.spawn(move || {
                barrier.wait();
                for commit_idx in 0..PER_THREAD {
                    let mut txn = shared.begin_write();
                    txn.mutator()
                        .create_node(
                            LabelSet::single(db_string("N")),
                            PropertyMap::from_pairs([(
                                db_string("k"),
                                selene_core::Value::Int(
                                    (thread_idx * PER_THREAD + commit_idx) as i64,
                                ),
                            )])
                            .unwrap(),
                        )
                        .unwrap();
                    let outcome = txn.commit().expect("commit ok");
                    assert!(outcome.durable_at.is_some());
                }
            });
        }
    });

    let total = THREADS * PER_THREAD;
    assert_eq!(durable.write_count(), total, "every commit appended once");
    assert_eq!(shared.read().node_count(), total);
    assert_eq!(shared.read().meta.generation, total as u64);
    // The win: far fewer flushes than writes. Worst case a flush per commit
    // (all serialized), best case ~total/64. Assert it actually grouped at all.
    assert!(
        durable.flush_count() < durable.write_count(),
        "ON grouped fsyncs: {} flushes for {} writes",
        durable.flush_count(),
        durable.write_count(),
    );
}

// ───────────────────────── T3 ─────────────────────────

/// Fan-out provider that records the published generation order it observes via
/// `on_change` of NodeCreated, proving publish order under batching.
struct GenOrderProvider {
    tag: ProviderTag,
    seen: Mutex<Vec<u64>>,
}

impl GenOrderProvider {
    fn new(tag: &[u8; 4]) -> Arc<Self> {
        Arc::new(Self {
            tag: ProviderTag(*tag),
            seen: Mutex::new(Vec::new()),
        })
    }
}

impl IndexProvider for GenOrderProvider {
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
            self.seen.lock().unwrap().push(id.get());
        }
        Ok(())
    }
    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

#[test]
fn t3_order_sensitive_batch_publishes_in_seal_order() {
    // Seal A,B,C each forking the prior, submit in reverse; assert final gen==3,
    // all present, and fan-out observed node ids in seal order (1,2,3).
    let durable = CountingDurable::new(b"CNT4");
    let provider = GenOrderProvider::new(b"GORD");
    let shared = Arc::new(graph_with_durable_and_provider(
        70_004,
        durable,
        provider.clone(),
        on(8, 8 * 1024 * 1024),
    ));

    let mut sealeds = Vec::new();
    let mut ids = Vec::new();
    for label in ["A", "B", "C"] {
        let mut txn = shared.begin_write();
        let id = txn
            .mutator()
            .create_node(LabelSet::single(db_string(label)), PropertyMap::new())
            .unwrap();
        ids.push(id);
        sealeds.push(txn.seal(None, None).expect("seals"));
    }

    // Submit C, B (reverse) on background threads, then A last to unblock.
    let sealed_c = sealeds.pop().unwrap();
    let sealed_b = sealeds.pop().unwrap();
    let sealed_a = sealeds.pop().unwrap();
    let s_c = Arc::clone(&shared);
    let c_thread = thread::spawn(move || s_c.submit_sealed_for_test(sealed_c).expect("C"));
    for _ in 0..1_000 {
        thread::yield_now();
    }
    let s_b = Arc::clone(&shared);
    let b_thread = thread::spawn(move || s_b.submit_sealed_for_test(sealed_b).expect("B"));
    for _ in 0..1_000 {
        thread::yield_now();
    }
    let outcome_a = shared.submit_sealed_for_test(sealed_a).expect("A");
    let outcome_b = b_thread.join().unwrap();
    let outcome_c = c_thread.join().unwrap();

    assert_eq!(outcome_a.generation, 1);
    assert_eq!(outcome_b.generation, 2);
    assert_eq!(outcome_c.generation, 3);
    let snap = shared.read();
    assert_eq!(snap.meta.generation, 3);
    assert_eq!(snap.node_count(), 3);
    // Fan-out observed publish order == seal order.
    let seen = provider.seen.lock().unwrap().clone();
    let expected: Vec<u64> = ids.iter().map(|id| id.get()).collect();
    assert_eq!(seen, expected, "fan-out observed seal-order publish");
}

// ───────────────────────── T4 ─────────────────────────

#[test]
fn t4_gap_ends_batch_no_deadlock() {
    // Allocate seqs 0,1,2; withhold 1. Submit 0 then 2. 0 publishes promptly; 2
    // stays buffered (never skip the gap). Releasing 1 publishes 1 then 2.
    let durable = CountingDurable::new(b"CNT5");
    let shared = Arc::new(graph_with_durable(70_005, durable, on(8, 8 * 1024 * 1024)));

    let mut s0 = shared.begin_write();
    s0.mutator()
        .create_node(LabelSet::single(db_string("Z0")), PropertyMap::new())
        .unwrap();
    let sealed0 = s0.seal(None, None).expect("0");
    let mut s1 = shared.begin_write();
    s1.mutator()
        .create_node(LabelSet::single(db_string("Z1")), PropertyMap::new())
        .unwrap();
    let sealed1 = s1.seal(None, None).expect("1");
    let mut s2 = shared.begin_write();
    s2.mutator()
        .create_node(LabelSet::single(db_string("Z2")), PropertyMap::new())
        .unwrap();
    let sealed2 = s2.seal(None, None).expect("2");

    // Submit 0 (publishes at once) and 2 (buffered behind the seq-1 gap).
    let outcome0 = shared.submit_sealed_for_test(sealed0).expect("0 publishes");
    assert_eq!(outcome0.generation, 1);

    let s_two = Arc::clone(&shared);
    let two_thread = thread::spawn(move || s_two.submit_sealed_for_test(sealed2).expect("2"));
    for _ in 0..1_000 {
        thread::yield_now();
    }
    // Gap not filled: only the seq-0 node is visible; the committer never skipped
    // the gap to publish seq 2.
    assert_eq!(shared.read().meta.generation, 1, "2 not published over gap");

    // Release the gap; 1 then 2 publish, no hang.
    let outcome1 = shared.submit_sealed_for_test(sealed1).expect("1 publishes");
    let outcome2 = two_thread.join().expect("2 thread ok");
    assert_eq!(outcome1.generation, 2);
    assert_eq!(outcome2.generation, 3);
    assert_eq!(shared.read().node_count(), 3);
}
