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
use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Record, Value, intern};
use selene_persist::{
    DEFAULT_WAL_FILE_NAME, SectionCompression, SnapshotConfig, SyncPolicy, WalConfig,
};

use selene_graph::{CommitBatching, SeleneGraph, SharedGraph, TypedIndexKind};

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

/// Persist a snapshot of `shared`'s live state at `sequence`.
///
/// Used by the snapshot-mid-stream arm so recovery must reconcile the snapshot
/// base with the post-snapshot WAL tail.
fn write_core_snapshot(dir: &Path, shared: &SharedGraph, sequence: u64) {
    let outcome = shared
        .write_snapshot(SnapshotConfig {
            dir: dir.to_path_buf(),
            sequence,
            compression: SectionCompression::None,
            fsync: false,
        })
        .unwrap();
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

// ---------------------------------------------------------------------------
// Coverage-followup #2: HEAVY-Value PropertyMaps + a closed-graph schema change
// through the full persist+graph recovery pipeline.
//
// `recovery_round_trips_arbitrary_op_sequence` (above) uses the shared harness's
// `arb_value`, which only emits Int/Float/String/Null. This focused test carries
// every heavy `Value` variant (all numeric widths, Decimal, String,
// Bytes, temporals, Uuid, nested List/Record) on a dedicated NON-indexed property
// key, commits a real `SchemaChange` (a property-index DDL — index admission for
// the indexed I64 key stays type-clean), snapshots + WALs through the funnel,
// drops, recovers, and asserts structural equality. The bar: would it catch a
// heavy-Value variant lost or mis-placed (the D22 row↔id hazard) through recovery?
// ---------------------------------------------------------------------------

/// The non-indexed property key the heavy values land on.
fn payload_key() -> IStr {
    intern("recover.heavy.payload").unwrap()
}

/// The indexed I64 key (only ever set to `Value::Int`, so index admission stays
/// type-clean while the schema change still exercises catalog DDL persistence).
fn indexed_key() -> IStr {
    intern("recover.heavy.age").unwrap()
}

fn heavy_label() -> IStr {
    intern("recover.heavy.node").unwrap()
}

fn heavy_edge_label() -> IStr {
    intern("recover.heavy.edge").unwrap()
}

/// Generate one heavy `Value`. Bounded `prop_recursive` covers the nested
/// `List` / `Record` containers; leaves cover every scalar/temporal variant.
fn heavy_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::Int),
        any::<u64>().prop_map(Value::Uint),
        any::<i128>().prop_map(Value::Int128),
        any::<u128>().prop_map(Value::Uint128),
        any::<f64>().prop_map(Value::Float),
        any::<f32>().prop_map(Value::Float32),
        any::<i64>().prop_map(|m| Value::Decimal(rust_decimal::Decimal::new(m, 2))),
        "[a-z]{1,8}".prop_map(|s| Value::String(intern(&format!("recover.v.{s}")).unwrap())),
        "[a-zA-Z0-9 ]{0,16}".prop_map(|s| Value::String(intern(&s).unwrap())),
        proptest::collection::vec(any::<u8>(), 0..20)
            .prop_map(|b| Value::Bytes(std::sync::Arc::from(b))),
        Just(Value::Date("2024-06-15".parse().unwrap())),
        Just(Value::LocalDateTime("2024-06-15T12:30:00".parse().unwrap())),
        Just(Value::LocalTime("12:30:00".parse().unwrap())),
        Just(Value::Duration(Box::new("PT3H15M".parse().unwrap()))),
        any::<u128>().prop_map(|n| Value::Uuid(uuid::Uuid::from_u128(n))),
        Just(Value::Null),
    ];
    leaf.prop_recursive(3, 12, 3, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..3).prop_map(Value::List),
            proptest::collection::vec(("[a-z]{1,5}", inner), 0..2).prop_map(|fields| {
                Value::Record(Box::new(Record::Open(
                    fields
                        .into_iter()
                        .map(|(k, v)| (intern(&format!("recover.f.{k}")).unwrap(), v))
                        .collect(),
                )))
            }),
        ]
    })
}

/// A property map with an Int on the indexed key and a heavy value on the
/// non-indexed payload key.
fn heavy_props() -> impl Strategy<Value = PropertyMap> {
    (0i64..50, heavy_value()).prop_map(|(age, payload)| {
        PropertyMap::from_pairs([(indexed_key(), Value::Int(age)), (payload_key(), payload)])
            .unwrap()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(16)
    ))]

    /// Commit a closed-graph schema change (property-index DDL) plus a run of
    /// node/edge creates carrying heavy-Value PropertyMaps through a WAL-backed
    /// funnel, drop, recover, and assert the recovered graph reproduces every
    /// alive id's exact content. Exercises the heavy `Value` postcard variants
    /// (numeric widths, Decimal, String, Bytes, temporals, Uuid, nested
    /// List/Record) end-to-end through WAL replay + index re-derivation — none of
    /// which the shared-harness `arb_value` (Int/Float/String/Null) reaches.
    #[test]
    fn recovery_round_trips_heavy_value_properties(
        node_props in proptest::collection::vec(heavy_props(), 1..=16),
        edge_props in proptest::collection::vec(heavy_props(), 0..=8),
    ) {
        for batching in [CommitBatching::Off, CommitBatching::DEFAULT_ON] {
            let dir = recovery_temp_dir();
            let graph_id = GraphId::new(911);
            let mut oracle = Oracle::default();

            {
                let shared = wal_backed_graph(&dir, graph_id, batching);
                // The closed-graph schema change: register an I64 index on the
                // indexed key. It commits a SchemaChange through the funnel that
                // must survive recovery alongside the heavy property rows.
                shared
                    .create_property_index(heavy_label(), indexed_key(), TypedIndexKind::I64)
                    .unwrap();

                let label_set = LabelSet::single(heavy_label());
                let mut created_nodes = Vec::new();
                // Create nodes carrying heavy props.
                for props in &node_props {
                    let id = {
                        let mut txn = shared.begin_write();
                        let id = txn
                            .mutator()
                            .create_node(label_set.clone(), props.clone())
                            .unwrap();
                        txn.commit().unwrap();
                        id
                    };
                    created_nodes.push(id);
                    oracle.nodes.push(id);
                    oracle.alive_nodes.insert(id);
                    oracle.node_labels.insert(id, label_set.clone());
                    oracle.node_props.insert(id, props.clone());
                }

                // Create edges between the heavy nodes, also carrying heavy props.
                for (i, props) in edge_props.iter().enumerate() {
                    let source = created_nodes[i % created_nodes.len()];
                    let target = created_nodes[(i + 1) % created_nodes.len()];
                    let id = {
                        let mut txn = shared.begin_write();
                        let id = txn
                            .mutator()
                            .create_edge(heavy_edge_label(), source, target, props.clone())
                            .unwrap();
                        txn.commit().unwrap();
                        id
                    };
                    oracle.edges.push(id);
                    oracle.alive_edges.insert(id);
                    oracle.edge_props.insert(id, props.clone());
                }
            }

            let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
            assert_recovered_matches_oracle(&recovered.read(), &oracle);
            let _ = fs::remove_dir_all(&dir);
        }
    }
}
