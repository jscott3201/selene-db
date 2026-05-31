//! GRAPH-09: randomized durability round-trip for the selene-graph mutation
//! funnel.
//!
//! Every other recovery test (`recover_tests`, `durable_round_trip`) is a
//! hand-crafted single scenario; this is the only randomized end-to-end
//! WAL→drop→recover proof. It drives a random op sequence through a WAL-backed
//! funnel, drops the graph (releasing the WAL lock), then `recover()`s and
//! asserts the recovered live state equals the oracle — every alive id, its
//! exact labels/properties (the GRAPH-10 content shadow), and self-consistent
//! re-derived indexes. The shared funnel harness + oracle live in
//! `funnel_harness` so they stay in lock-step with the in-memory consistency
//! suite in `property_tests.rs`.

mod funnel_harness;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use proptest::prelude::*;
use selene_core::GraphId;
use selene_persist::{
    DEFAULT_WAL_FILE_NAME, SectionCompression, SnapshotBuilder, SnapshotConfig, SyncPolicy,
    WalConfig,
};

use selene_graph::{CORE_PROVIDER_TAG, CommitBatching, ProviderTag, SeleneGraph, SharedGraph};

use funnel_harness::{Oracle, apply_op, arb_op, assert_snapshot_matches_oracle};

/// Create a fresh, unique temp directory for one recovery round-trip case.
fn recovery_temp_dir() -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "selene-graph-recover-prop-{}-{nanos}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build a WAL-backed open graph under the given batching policy at `dir`.
fn wal_backed_graph(dir: &Path, graph_id: GraphId, batching: CommitBatching) -> SharedGraph {
    SharedGraph::builder(graph_id)
        .with_wal(dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default())
        .unwrap()
        .with_commit_batching(batching)
        .build()
        .unwrap()
}

/// Persist a `CORE/*` snapshot of `shared`'s live state at `sequence`, mirroring
/// the production CORE provider's section writes (same path the recover_tests
/// `write_snapshot` helper exercises). Used by the snapshot-mid-stream arm so
/// recovery must reconcile the snapshot base with the post-snapshot WAL tail.
fn write_core_snapshot(dir: &Path, shared: &SharedGraph, sequence: u64) {
    let provider = shared
        .index_provider_by_tag(ProviderTag(CORE_PROVIDER_TAG))
        .expect("core provider registered");
    let mut builder = SnapshotBuilder::new(SnapshotConfig {
        dir: dir.to_path_buf(),
        sequence,
        compression: SectionCompression::None,
        fsync: false,
    });
    for sub in provider.declared_sub_tags() {
        let bytes = provider.write_section(*sub).unwrap();
        builder
            .add_section(CORE_PROVIDER_TAG, sub.0, bytes)
            .unwrap();
    }
    let outcome = builder.finalize().unwrap();
    assert_eq!(outcome.snapshot_seq, sequence);
}

/// Assert the recovered graph reproduces every alive id + its exact content, and
/// that its re-derived indexes are internally consistent. This is the durability
/// half of GRAPH-10: a recovery that placed a row at the wrong position or
/// dropped a property would pass liveness/count parity but fail content equality.
fn assert_recovered_matches_oracle(recovered: &SeleneGraph, oracle: &Oracle) {
    recovered
        .assert_indexes_consistent()
        .expect("recovered indexes consistent");
    assert_snapshot_matches_oracle(recovered, oracle);
}

proptest! {
    // Recovery round-trips do real filesystem I/O (WAL append + reopen +
    // snapshot) per case, so this proptest is capped well below the 200-case
    // funnel default to keep wall-time bounded; PROPTEST_CASES still scales it.
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(24)
    ))]

    /// A random op sequence committed through a WAL-backed funnel must recover
    /// bit-for-bit (alive set + content + index consistency). Covered under both
    /// batching policies (Off fsyncs per commit, On coalesces a run behind one
    /// barrier) plus a snapshot-mid-stream arm so the snapshot-base + WAL-tail
    /// reconciliation path is also exercised. Fails if WAL replay placed a row at
    /// the wrong position (the D22 row↔id hazard surviving the recover boundary),
    /// dropped/duplicated a change, or lost a property — none of which the
    /// in-memory funnel proptest can see because it never reopens.
    #[test]
    fn recovery_round_trips_arbitrary_op_sequence(
        first in proptest::collection::vec(arb_op(), 1..=40),
        second in proptest::collection::vec(arb_op(), 0..=20),
    ) {
        for batching in [CommitBatching::Off, CommitBatching::DEFAULT_ON] {
            let dir = recovery_temp_dir();
            let graph_id = GraphId::new(909);
            let mut oracle = Oracle::default();
            // Build, commit the whole sequence through the WAL, then drop to
            // release the writer lock before recovery reopens the file.
            {
                let shared = wal_backed_graph(&dir, graph_id, batching);
                for (i, op) in first.iter().chain(second.iter()).enumerate() {
                    apply_op(&shared, &mut oracle, op, i);
                }
            }
            let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
            assert_recovered_matches_oracle(&recovered.read(), &oracle);
            let _ = fs::remove_dir_all(&dir);
        }

        // Snapshot-mid-stream arm: replay `first` into a no-WAL graph and snapshot
        // it at its generation (so the snapshot's live ids define the floor), then
        // recover from the snapshot, commit `second` through the reopened WAL (its
        // entries land strictly above the floor), and recover again. The final
        // recover must reconcile the snapshot base with the post-snapshot WAL tail
        // — mirroring production rotate-on-snapshot (snapshot at N ⇒ WAL floor N ⇒
        // tail at N+1..).
        {
            let dir = recovery_temp_dir();
            let graph_id = GraphId::new(910);
            let mut oracle = Oracle::default();

            // (1) Replay `first` into a WAL-less graph and snapshot it.
            let base = SharedGraph::new(graph_id);
            for (i, op) in first.iter().enumerate() {
                apply_op(&base, &mut oracle, op, i);
            }
            let snapshot_seq = base.read().meta.generation;
            write_core_snapshot(&dir, &base, snapshot_seq);

            // (2) Seed an empty WAL anchored at the snapshot floor so recover()'s
            // legacy cross-check (reader.snapshot_seq() == applied_snapshot_seq)
            // is satisfied and replay starts at snapshot_seq + 1; recover; commit
            // the second batch through the reopened WAL.
            {
                drop(
                    SharedGraph::builder(graph_id)
                        .with_wal(
                            dir.join(DEFAULT_WAL_FILE_NAME),
                            WalConfig {
                                sync_policy: SyncPolicy::OnFlushOnly,
                                snapshot_seq,
                            },
                        )
                        .unwrap()
                        .build()
                        .unwrap(),
                );
                let live = SharedGraph::recover(&dir, graph_id).unwrap();
                for (i, op) in second.iter().enumerate() {
                    apply_op(&live, &mut oracle, op, snapshot_seq as usize + 1 + i);
                }
            }

            // (3) Final recover must reconcile snapshot base + WAL tail == oracle.
            let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
            assert_recovered_matches_oracle(&recovered.read(), &oracle);
            let _ = fs::remove_dir_all(&dir);
        }
    }
}
