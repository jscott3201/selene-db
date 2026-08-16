use super::*;

// ───────────────────────── T5 (load-bearing) ─────────────────────────

#[test]
fn t5_partial_batch_append_failure_errs_all_and_poisons() {
    // A durable that Errs on its 3rd write_commit. Fan in 5 contiguous commits
    // (seal_seq 0..4) under On(8). The 3rd append (seal_seq 2) fails, so NONE
    // publish, ALL 5 reply Err (including the 2 already-appended members 0,1),
    // poisoned, no hang, later commit fails fast.
    //
    // DETERMINISM: the failure outcome depends on all 5 forming ONE contiguous
    // batch. If seals 0/1 drained alone (before the later seqs arrive) they would
    // publish+ack OK and only 2,3,4 would Err — a real, correct engine behavior
    // that makes a naive "submit all 5 from racing threads" assertion flaky
    // (reproduced err_count==3 and ==4 under scheduling load). So we use the gap
    // technique (mirrors T3/T4/T7): submit the LATER seqs (b,c,d,e == seal_seq
    // 1..4) FIRST on background threads — they buffer behind the seq-0 gap in the
    // committer's reorder buffer and CANNOT publish — then submit seq 0 (a) LAST.
    // When a arrives the full contiguous run [0,1,2,3,4] is present, so
    // drain_contiguous_batch forms one batch: append 0 (write #1 ok), append 1
    // (write #2 ok), append 2 (write #3 FAILS) ⇒ AppendFailed{[0,1]} with seq 2
    // Err'd inline, seqs 0,1 Err'd via ack_appended_with_error, seqs 3,4 via
    // drain_buffer_with_error ⇒ deterministically err_count == 5, generation == 0.
    let durable = CountingDurable::fail_write_on(b"CNT6", 3);
    let shared = Arc::new(graph_with_durable(
        70_006,
        durable.clone(),
        on(8, 8 * 1024 * 1024),
    ));

    // Seal 5 commits so they form one contiguous run when submitted together.
    let mut sealeds = Vec::new();
    for label in ["a", "b", "c", "d", "e"] {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(db_string(label)), PropertyMap::new())
            .unwrap();
        sealeds.push(txn.seal(None, None).expect("seals"));
    }

    // seal_seq 0 (a) is withheld; enqueue seal_seq 1..4 (e,d,c,b in pop order)
    // first so they buffer behind the seq-0 gap, then unblock with seq 0 last.
    // Use the async test seam here: the production submit path blocks waiting
    // for its reply, so background threads can race seq 0 on Linux before every
    // later seq is actually enqueued. Same-thread sends are FIFO and make the
    // intended [0,1,2,3,4] run deterministic.
    let sealed_a = sealeds.remove(0);
    let mut replies = Vec::new();
    while let Some(sealed) = sealeds.pop() {
        replies.push(
            shared
                .submit_sealed_async_for_test(sealed)
                .expect("later seq enqueued"),
        );
    }
    // Seq 0 arrives last: the full [0,1,2,3,4] contiguous run is now buffered, so
    // it drains as ONE batch and the 3rd append fails the whole run.
    let a_reply = shared
        .submit_sealed_async_for_test(sealed_a)
        .expect("seq 0 enqueued");
    let a_result = a_reply
        .recv_timeout(Duration::from_secs(10))
        .expect("seq 0 waiter did not hang");
    let mut err_count = usize::from(a_result.is_err());
    for reply in replies {
        let result = reply
            .recv_timeout(Duration::from_secs(10))
            .expect("later waiter did not hang");
        if result.is_err() {
            err_count += 1;
        }
    }
    assert_eq!(err_count, 5, "all 5 members Err (incl. already-appended)");

    // Nothing published: generation never advanced.
    assert_eq!(shared.read().meta.generation, 0);
    assert_eq!(shared.read().node_count(), 0);

    // Engine poisoned: a later commit fails fast.
    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(LabelSet::new(), PropertyMap::new())
        .unwrap();
    assert!(
        matches!(txn.commit(), Err(GraphError::IndeterminateCommit { .. })),
        "post-poison commit fails fast, and as indeterminate: committer_dead \
         cannot know whether this caller's record reached the WAL",
    );
}

// ───────────────────────── T6 ─────────────────────────

#[test]
fn t6_flush_failure_poisons_all() {
    // write_commit Ok but flush() Err. Submit 4 contiguous. NONE publish, all 4
    // Err, poisoned, bounded.
    let durable = CountingDurable::fail_flush(b"CNT7");
    let shared = Arc::new(graph_with_durable(
        70_007,
        durable.clone(),
        on(8, 8 * 1024 * 1024),
    ));

    let mut sealeds = Vec::new();
    for label in ["a", "b", "c", "d"] {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(db_string(label)), PropertyMap::new())
            .unwrap();
        sealeds.push(txn.seal(None, None).expect("seals"));
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut handles = Vec::new();
    for sealed in sealeds {
        let shared = Arc::clone(&shared);
        handles.push(thread::spawn(move || shared.submit_sealed_for_test(sealed)));
    }
    let mut err_count = 0;
    for handle in handles {
        if handle.join().expect("no panic").is_err() {
            err_count += 1;
        }
    }
    assert!(Instant::now() < deadline, "no waiter hung");
    assert_eq!(err_count, 4, "all 4 Err on flush failure");
    assert_eq!(shared.read().meta.generation, 0, "nothing published");
    // No flush ever succeeded.
    assert_eq!(durable.flush_count(), 0);
}

// ───────────────────────── T5b (publish-tail panic) ─────────────────────────

/// Fan-out provider that, on the FIRST `NodeCreated` it observes (member 1's
/// publish, on the committer thread), arms the test-only Stage-3 publish-panic
/// injection so the NEXT `publish_appended` (member 2) panics. This is the only
/// way to drive the committer's multi-member publish-panic poison-and-drain
/// branch: a provider `on_change` panic is swallowed by `notify_providers`, so
/// the panic must come from `publish_appended` itself.
struct ArmPublishPanicProvider {
    tag: ProviderTag,
    armed: AtomicBool,
}

impl ArmPublishPanicProvider {
    fn new(tag: &[u8; 4]) -> Arc<Self> {
        Arc::new(Self {
            tag: ProviderTag(*tag),
            armed: AtomicBool::new(false),
        })
    }
}

impl IndexProvider for ArmPublishPanicProvider {
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
        // Arm exactly once, during member 1's publish fan-out, so member 2's
        // publish_appended panics (arm(1) ⇒ the very next maybe_panic fires).
        if matches!(change, Change::NodeCreated { .. }) && !self.armed.swap(true, Ordering::SeqCst)
        {
            crate::write_txn::publish_panic_inject::arm(1);
        }
        Ok(())
    }
    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

#[test]
fn t5b_publish_tail_panic_acks_member_errs_rest_and_poisons() {
    // BRIEF 2 crash matrix item 6: a Stage-3 publish panic mid-batch. Form a
    // deterministic 3-member contiguous run [0,1,2]. Member 0 publishes OK
    // (visible + acked) and, via its fan-out, arms the injection so member 1's
    // publish_appended panics. The committer's catch_unwind poisons, Errs member 1
    // (its reply was already taken), then ack_appended_with_error Errs the
    // remaining member 2, drains the buffer, and stops — no dropped SyncSender, no
    // hang. Member 0 stays committed (durable-before-visible: it flushed + stored
    // before the panic); members 1,2 Err.
    let durable = CountingDurable::new(b"CNT8");
    let provider = ArmPublishPanicProvider::new(b"ARMP");
    let shared = Arc::new(graph_with_durable_and_provider(
        70_008,
        durable,
        provider,
        on(8, 8 * 1024 * 1024),
    ));

    // Seal 3 commits (seal_seq 0,1,2).
    let mut sealeds = Vec::new();
    for label in ["x", "y", "z"] {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(db_string(label)), PropertyMap::new())
            .unwrap();
        sealeds.push(txn.seal(None, None).expect("seals"));
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    // Buffer seqs 2,1 behind the seq-0 gap, then release seq 0 last so the full
    // [0,1,2] run drains as ONE batch (member 1's publish panics inside it).
    let sealed_0 = sealeds.remove(0);
    let mut handles = Vec::new();
    while let Some(sealed) = sealeds.pop() {
        let shared = Arc::clone(&shared);
        handles.push(thread::spawn(move || shared.submit_sealed_for_test(sealed)));
        for _ in 0..1_000 {
            thread::yield_now();
        }
    }
    let r0 = shared.submit_sealed_for_test(sealed_0);
    let mut later = Vec::new();
    for handle in handles {
        later.push(handle.join().expect("waiter thread did not panic"));
    }
    assert!(Instant::now() < deadline, "no waiter hung");

    // Member 0 committed (acked Ok, visible); members 1,2 Err'd (panic + drain).
    assert!(
        r0.is_ok(),
        "member 0 published before the panic, got {r0:?}"
    );
    assert_eq!(r0.unwrap().generation, 1);
    assert_eq!(
        later.iter().filter(|r| r.is_err()).count(),
        2,
        "the panicking member + the remaining member both Err",
    );

    // Exactly member 0 is visible; the diverged 1,2 never published.
    assert_eq!(shared.read().node_count(), 1);
    assert_eq!(shared.read().meta.generation, 1);

    // Engine poisoned: a later commit fails fast, no hang.
    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(LabelSet::new(), PropertyMap::new())
        .unwrap();
    assert!(
        matches!(txn.commit(), Err(GraphError::IndeterminateCommit { .. })),
        "post-panic commit fails fast (engine poisoned), as indeterminate",
    );
}
