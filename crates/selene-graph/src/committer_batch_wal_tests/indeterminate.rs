//! Evidence for [`GraphError::IndeterminateCommit`] — commits that were
//! error-acked and are present anyway after a reopen.
//!
//! Three doc comments in the commit path used to assert that a poison exit's
//! appended-but-unflushed bytes are "correct to lose on reopen". That is true of
//! losing the page cache, which is a machine crash. Poisoning the committer is
//! not a crash: it requires the embedder to drop the handle and recover, and the
//! bytes are still there. Under
//! [`SyncPolicy::OnFlushOnly`](selene_persist::SyncPolicy) `append_record`
//! hands the kernel one complete, checksum-valid frame, and recovery's tail
//! repair truncates only a torn tail — which such a frame is not.
//!
//! Note that `SyncPolicy::OnFlushOnly` does not even fsync on drop
//! (`WalWriter::syncs_on_drop` is `EveryN`-only), so nothing in these tests
//! makes the records durable. They survive purely because `write_all` put them
//! in the page cache, which is exactly the mechanism under test.
//!
//! These tests pin the CURRENT behaviour: an error-acked commit IS recovered.
//! That is what makes `40003` (*transaction rollback — statement completion
//! unknown*, ISO §23.1 Table 8) the honest status rather than a relabel of
//! `5GQL0`. Truncating the WAL back to a flushed watermark on the poison exit
//! would make the appended-not-flushed cases genuinely absent and let them
//! report a definite `40000`; when that lands, the first test below inverts and
//! the second splits.

use super::*;

use selene_persist::WalWriter;

/// A WAL-backed graph that also carries a synthetic durable provider.
///
/// The public [`SharedGraph::builder`] exposes no synthetic-durable seam, so
/// this goes through the crate-internal constructor. Provider order is
/// load-bearing: `from_graph_with_core_and_durables` pushes the core WAL
/// provider LAST, so a synthetic failure fires before the WAL provider's turn
/// on that member — while every earlier member of the same run has already been
/// written.
fn wal_graph_with_durable(
    dir: &std::path::Path,
    id: GraphId,
    durable: Arc<dyn DurableProvider>,
    batching: CommitBatching,
) -> SharedGraph {
    let writer = WalWriter::open(
        &dir.join(DEFAULT_WAL_FILE_NAME),
        WalConfig {
            // What `shared::builder::open_fresh_wal` forces unconditionally;
            // spelled out here because it is the premise of these tests.
            sync_policy: SyncPolicy::OnFlushOnly,
            ..WalConfig::default()
        },
    )
    .expect("wal opens");
    SharedGraph::from_graph_with_core_and_durables(
        SeleneGraph::new(id),
        Vec::new(),
        vec![durable],
        Some(writer),
        None,
        batching,
    )
    .expect("graph builds with synthetic durable + real WAL")
}

/// A group-flush failure Errs the commit, and the commit is there afterwards.
///
/// `flush_durables` short-circuits on the first failing provider, so the
/// synthetic one fails before the core WAL provider is ever asked to fsync —
/// yet the record was written during the append phase and outlives the reopen.
#[test]
fn a_flush_failure_errs_a_commit_whose_bytes_survive_the_reopen() {
    let dir = temp_dir("indeterminate-flush");
    let id = GraphId::new(70_090);

    let error = {
        let shared = wal_graph_with_durable(
            &dir,
            id,
            CountingDurable::fail_flush(b"IND1"),
            CommitBatching::Off,
        );
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(db_string("Survivor")), PropertyMap::new())
            .unwrap();
        let error = txn.commit().expect_err("the group flush fails");
        assert_eq!(
            shared.read().node_count(),
            0,
            "nothing publishes from a failed run, so the caller's own handle \
             never shows it — which is why the reopen below is a surprise"
        );
        error
        // Dropped here: the committer joins and the WAL writer closes without
        // fsync (OnFlushOnly does not sync on drop).
    };

    assert!(
        matches!(error, GraphError::IndeterminateCommit { .. }),
        "a poison exit cannot promise the changes were canceled, got {error:?}"
    );
    assert_eq!(
        error.gqlstatus(),
        "40003",
        "ISO 23.1 Table 8: transaction rollback - statement completion unknown"
    );

    let recovered = SharedGraph::recover(&dir, id).expect("recovers");
    assert_eq!(
        recovered.read().node_count(),
        1,
        "the Err'd commit is PRESENT after the mandated reopen. A caller that \
         read that Err as 'this did not happen' and re-drove the write would \
         double-apply it -- which is the defect #1091 was filed for."
    );
}

/// The consumer's scenario: a partial-batch append failure Errs five commits and
/// two of them come back.
///
/// Staged exactly like `t5_partial_batch_append_failure_errs_all_and_poisons`,
/// but over a real WAL so recovery can be observed. Members 0 and 1 are appended
/// (synthetic writes 1 and 2 succeed, so the core WAL provider writes each
/// record), member 2's synthetic write fails, and every one of the five waiters
/// is error-acked.
#[test]
fn a_partial_batch_append_failure_leaves_earlier_members_in_the_wal() {
    let dir = temp_dir("indeterminate-append");
    let id = GraphId::new(70_091);
    const MEMBERS: usize = 5;

    let mut errors = Vec::with_capacity(MEMBERS);
    {
        let shared = Arc::new(wal_graph_with_durable(
            &dir,
            id,
            CountingDurable::fail_write_on(b"IND2", 3),
            on(8, 8 * 1024 * 1024),
        ));

        let mut sealeds = Vec::new();
        for label in ["a", "b", "c", "d", "e"] {
            let mut txn = shared.begin_write();
            txn.mutator()
                .create_node(LabelSet::single(db_string(label)), PropertyMap::new())
                .unwrap();
            sealeds.push(txn.seal(None, None).expect("seals"));
        }

        // Withhold seal_seq 0 so 1..4 buffer behind the gap, then release 0 to
        // form one contiguous run of five. Same-thread sends are FIFO, which is
        // what makes the run deterministic.
        let sealed_a = sealeds.remove(0);
        let mut replies = Vec::new();
        while let Some(sealed) = sealeds.pop() {
            replies.push(
                shared
                    .submit_sealed_async_for_test(sealed)
                    .expect("later seq enqueued"),
            );
        }
        let a_reply = shared
            .submit_sealed_async_for_test(sealed_a)
            .expect("seq 0 enqueued");
        replies.push(a_reply);

        for reply in replies {
            let result = reply
                .recv_timeout(Duration::from_secs(10))
                .expect("waiter did not hang");
            errors.push(result.expect_err("every member of a failed run is Err'd"));
        }
        assert_eq!(shared.read().meta.generation, 0, "nothing published");
    }

    assert_eq!(errors.len(), MEMBERS);
    for error in &errors {
        assert!(
            matches!(error, GraphError::IndeterminateCommit { .. }),
            "every waiter on the poison exit gets the indeterminate outcome, \
             including the ones whose records reached the WAL, got {error:?}"
        );
    }

    let recovered = SharedGraph::recover(&dir, id).expect("recovers");
    assert_eq!(
        recovered.read().node_count(),
        2,
        "members 0 and 1 were appended before member 2's write failed, so two \
         of the five Err'd commits are live after recovery. Five callers were \
         told the write failed; two of the writes are in the graph."
    );
}
