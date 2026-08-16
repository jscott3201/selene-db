//! A recovered graph reports the torn WAL tail it discarded (#1109).
//!
//! `SharedGraph::recover` returning `Ok` does not mean nothing was lost. A
//! final frame that is short, corrupt, or zero-filled was never acknowledged,
//! so discarding exactly it is correct crash recovery — but a commit a client
//! believed it had submitted may have been inside it. Before this, the only
//! trace was a `tracing::warn!` an embedder with no subscriber never sees, and
//! `recover` returns `GraphResult<Self>` with nowhere to put an outcome.
//!
//! The load-bearing assertion in each test below is not that a repair is
//! *reported* but that the report **matches what was actually lost**: the
//! recovered node count drops by exactly the truncated commit, and the reported
//! offset is exactly the frame boundary the test truncated into. A flag that
//! said "something happened" without those would be no more actionable than the
//! log line it replaces.

use std::fs;

use super::*;
use crate::WalTailReason;

/// Commit `count` single-node transactions through a WAL-backed graph, closing
/// it afterwards, and return the WAL file length after each commit.
///
/// Each returned length is a frame boundary: the committer runs
/// `CommitBatching::Off` under `SyncPolicy::OnFlushOnly`, so one `commit()` is
/// one append plus one flush and the bytes are on disk when it returns.
fn seed_commits(dir: &Path, graph_id: GraphId, count: usize) -> Vec<u64> {
    let wal_path = dir.join(DEFAULT_WAL_FILE_NAME);
    let shared = SharedGraph::builder(graph_id)
        .with_wal(&wal_path, WalConfig::default())
        .unwrap()
        .build()
        .unwrap();
    let mut boundaries = Vec::with_capacity(count);
    for index in 0..count {
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node(
                    LabelSet::single(db_string("tail.repair").unwrap()),
                    prop("seq", Value::Int(index as i64)),
                )
                .unwrap();
        }
        txn.commit().unwrap();
        boundaries.push(fs::metadata(&wal_path).unwrap().len());
    }
    drop(shared);
    boundaries
}

/// The reported repair names the frame that was cut, and the graph comes back
/// missing exactly that commit.
#[test]
fn a_recovered_graph_reports_the_tail_it_discarded() {
    let dir = temp_dir("tail-repair-reported");
    let graph_id = GraphId::new(91);
    let boundaries = seed_commits(&dir, graph_id, 3);
    let final_frame_start = boundaries[1];

    // Cut 8 bytes into the final frame. That is inside its fixed prefix, so the
    // file ends part-way through a frame rather than after a complete one.
    let wal_path = dir.join(DEFAULT_WAL_FILE_NAME);
    fs::OpenOptions::new()
        .write(true)
        .open(&wal_path)
        .unwrap()
        .set_len(final_frame_start + 8)
        .unwrap();

    let recovered =
        SharedGraph::recover(&dir, graph_id).expect("a torn tail is repaired, not a failure");

    let repair = recovered
        .recovery_tail_repair()
        .expect("recovery discarded a tail and must say so");
    assert_eq!(repair.reason, WalTailReason::ShortFrame);
    assert_eq!(
        repair.offset, final_frame_start,
        "the report must name the frame that was cut, not merely that one was"
    );
    assert_eq!(repair.discarded_bytes, 8);

    // The point of reporting: a commit really is gone. Two survive, not three.
    assert_eq!(
        recovered.read().node_count(),
        2,
        "the discarded frame carried the third commit"
    );
    drop(recovered);
}

/// An intact WAL reports nothing, so the report discriminates rather than
/// firing on every reopen.
#[test]
fn an_intact_wal_reports_no_repair() {
    let dir = temp_dir("tail-repair-absent");
    let graph_id = GraphId::new(92);
    seed_commits(&dir, graph_id, 3);

    let recovered = SharedGraph::recover(&dir, graph_id).expect("an intact WAL recovers");
    assert!(
        recovered.recovery_tail_repair().is_none(),
        "an intact WAL must not report a repair"
    );
    assert_eq!(recovered.read().node_count(), 3);
    drop(recovered);
}

/// A graph that was built rather than recovered has no tail to repair, and the
/// accessor says so instead of carrying a stale value from some other reopen.
#[test]
fn a_built_graph_reports_no_repair() {
    let dir = temp_dir("tail-repair-built");
    let graph_id = GraphId::new(93);
    let shared = SharedGraph::builder(graph_id)
        .with_wal(dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default())
        .unwrap()
        .build()
        .unwrap();
    assert!(shared.recovery_tail_repair().is_none());
    drop(shared);
}

/// The report survives the recovery variants, not just the bare entry point:
/// all four route through `recover_inner`, and a future refactor that stamped
/// only `recover` would leave the other three silently lying.
#[test]
fn the_provider_recovery_variant_reports_the_same_repair() {
    let dir = temp_dir("tail-repair-variant");
    let graph_id = GraphId::new(94);
    let boundaries = seed_commits(&dir, graph_id, 3);

    let wal_path = dir.join(DEFAULT_WAL_FILE_NAME);
    fs::OpenOptions::new()
        .write(true)
        .open(&wal_path)
        .unwrap()
        .set_len(boundaries[1] + 8)
        .unwrap();

    let recovered = SharedGraph::recover_with_providers(&dir, graph_id, Vec::new())
        .expect("a torn tail is repaired, not a failure");
    let repair = recovered
        .recovery_tail_repair()
        .expect("the provider variant must report the tail too");
    assert_eq!(repair.reason, WalTailReason::ShortFrame);
    assert_eq!(repair.offset, boundaries[1]);
    assert_eq!(recovered.read().node_count(), 2);
    drop(recovered);
}
