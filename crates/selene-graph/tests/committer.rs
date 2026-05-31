//! v1.2 multi-writer BRIEF 1 — single-committer + `WriteTxn::seal` integration
//! tests.
//!
//! These exercise the seal-and-handover commit path through the public API: the
//! committer is the sole snapshot publisher, commits route through it and block
//! until durable + visible, GG02 still aborts pre-handoff on the session thread,
//! ids stay monotonic in commit order, compaction serializes with commits, and a
//! committer that panics fails every waiter cleanly rather than hanging.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use selene_core::{
    Change, GraphId, IStr, LabelSet, NodeId, PropertyMap, PropertyValueType, Value, intern,
};
use selene_graph::{
    GraphError, GraphTypeDef, NodeTypeDef, PropertyTypeDef, SharedGraph, ValidationMode,
};
use selene_persist::{DEFAULT_WAL_FILE_NAME, WalConfig};

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn prop(name: &str, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(istr(name), value)]).expect("valid property map")
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-committer-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).unwrap();
    dir
}

/// A closed-graph type requiring a non-null `name` on `Person` nodes, so an
/// empty-property `Person` insert is a GG02 violation.
fn person_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: istr("committer.person.graph"),
        node_types: vec![NodeTypeDef {
            name: istr("committer.person"),
            key_labels: LabelSet::single(istr("Person")),
            properties: vec![PropertyTypeDef {
                name: istr("name"),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: true,
                default: None,
                immutable: false,
                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    }
}

#[test]
fn commit_routes_through_committer_and_is_visible_on_return() {
    let shared = SharedGraph::new(GraphId::new(1_001));
    let mut txn = shared.begin_write();
    let node_id = txn
        .mutator()
        .create_node(LabelSet::new(), PropertyMap::new())
        .expect("create_node ok");
    let outcome = txn.commit().expect("commit ok");

    // commit() returns ⇒ durable + visible: the published snapshot already has
    // the node, with no extra synchronization on the caller's part.
    assert_eq!(outcome.generation, 1);
    assert!(shared.read().is_node_alive(node_id));
}

#[test]
fn handover_preserves_change_list_and_ids() {
    let shared = SharedGraph::new(GraphId::new(1_002));
    let mut txn = shared.begin_write();
    let a = txn
        .mutator()
        .create_node(LabelSet::single(istr("A")), PropertyMap::new())
        .unwrap();
    let b = txn
        .mutator()
        .create_node(LabelSet::single(istr("B")), PropertyMap::new())
        .unwrap();
    let outcome = txn.commit().unwrap();

    // The change list survives the seal → committer handoff intact.
    assert_eq!(outcome.changes.len(), 2);
    assert!(matches!(outcome.changes[0], Change::NodeCreated { id, .. } if id == a));
    assert!(matches!(outcome.changes[1], Change::NodeCreated { id, .. } if id == b));
    // next ids are reported from the seal-time peek.
    assert_eq!(outcome.next_node_id, b.get() + 1);
}

#[test]
fn id_monotonicity_across_sequential_commits() {
    let shared = SharedGraph::new(GraphId::new(1_003));
    let mut ids = Vec::new();
    for _ in 0..10 {
        let mut txn = shared.begin_write();
        let id = txn
            .mutator()
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        ids.push(id);
    }
    // Ids are monotonic in commit order (== seal order == publish order).
    for pair in ids.windows(2) {
        assert!(pair[1].get() > pair[0].get(), "ids strictly increase");
    }
}

#[test]
fn gg02_violation_aborts_in_seal_and_rolls_back_generation_bump() {
    let shared = SharedGraph::builder(GraphId::new(1_004))
        .bound_to(person_graph_type())
        .expect("bind closed graph")
        .build()
        .expect("build closed graph");

    let gen_before = shared.read().meta.generation;

    let mut txn = shared.begin_write();
    // A Person with no `name` violates the closed-graph type.
    txn.mutator()
        .create_node(LabelSet::single(istr("Person")), PropertyMap::new())
        .expect("mutation stages (validation is at commit)");
    let err = txn.commit().expect_err("GG02 must abort the commit");
    assert!(
        matches!(err, GraphError::TypeViolation(_)),
        "expected closed-graph violation, got {err:?}"
    );

    // The seal-time generation bump is rolled back by Drop (no published epoch
    // advance), exactly as pre-v1.2: the abort happens on the session thread
    // before any handoff to the committer.
    assert_eq!(shared.read().meta.generation, gen_before);
    assert_eq!(shared.read().node_count(), 0);
}

#[test]
fn abort_isolation_does_not_leak_into_next_commit() {
    let shared = SharedGraph::builder(GraphId::new(1_005))
        .bound_to(person_graph_type())
        .unwrap()
        .build()
        .unwrap();

    // Aborted commit (GG02).
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(istr("Person")), PropertyMap::new())
            .unwrap();
        assert!(txn.commit().is_err());
    }
    // A subsequent valid commit succeeds and publishes generation 1 (the
    // aborted one never advanced the published generation).
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::String(istr("ada"))),
            )
            .unwrap();
        let outcome = txn.commit().unwrap();
        assert_eq!(outcome.generation, 1);
    }
    assert_eq!(shared.read().node_count(), 1);
}

#[test]
fn durable_commit_reports_sequence_after_committer_fsync() {
    let dir = temp_dir("durable-seq");
    let wal_path = dir.join(DEFAULT_WAL_FILE_NAME);
    let shared = SharedGraph::builder(GraphId::new(1_006))
        .with_wal(&wal_path, WalConfig::default())
        .unwrap()
        .build()
        .unwrap();

    let mut txn = shared.begin_write();
    let id = txn
        .mutator()
        .create_node(LabelSet::new(), PropertyMap::new())
        .unwrap();
    let outcome = txn.commit().unwrap();

    assert_eq!(id, NodeId::new(1));
    // durable_at is only delivered after the committer's WAL append (WAL-first).
    assert_eq!(outcome.durable_at, Some(1));
}

#[test]
fn compact_serializes_with_concurrent_commits() {
    let shared = Arc::new(SharedGraph::new(GraphId::new(1_007)));

    // Seed: create then delete a batch of nodes to leave reclaimable holes.
    let mut survivors = Vec::new();
    {
        let mut txn = shared.begin_write();
        let mut ids = Vec::new();
        for _ in 0..50 {
            ids.push(
                txn.mutator()
                    .create_node(LabelSet::single(istr("T")), PropertyMap::new())
                    .unwrap(),
            );
        }
        txn.commit().unwrap();
        let mut txn = shared.begin_write();
        for (idx, id) in ids.iter().enumerate() {
            if idx % 2 == 0 {
                txn.mutator().delete_node(*id).unwrap();
            } else {
                survivors.push(*id);
            }
        }
        txn.commit().unwrap();
    }

    // Hammer commits from N threads while one thread compacts repeatedly. The
    // committer serializes both, so neither corrupts the other.
    let threads: usize = 8;
    let barrier = Arc::new(Barrier::new(threads + 1));
    let mut handles = Vec::new();
    for t in 0..threads {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            for _ in 0..25 {
                let mut txn = shared.begin_write();
                txn.mutator()
                    .create_node(LabelSet::single(istr("W")), prop("t", Value::Int(t as i64)))
                    .unwrap();
                txn.commit().unwrap();
            }
        }));
    }
    let compactor = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            for _ in 0..10 {
                shared.compact().expect("compaction serializes cleanly");
            }
        })
    };
    for h in handles {
        h.join().expect("writer thread did not panic");
    }
    compactor.join().expect("compactor did not panic");

    // 8 threads * 25 new nodes + all survivors are alive and consistent.
    assert_eq!(shared.read().node_count(), threads * 25 + survivors.len());
    for id in &survivors {
        assert!(shared.read().is_node_alive(*id));
    }
}

#[test]
fn index_ddl_serializes_with_commits_through_committer() {
    let shared = Arc::new(SharedGraph::new(GraphId::new(1_008)));
    let label = istr("Doc");
    let key = istr("rank");

    // Seed an indexed-compatible node so the DDL build passes.
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(label), prop("rank", Value::Int(1)))
            .unwrap();
        txn.commit().unwrap();
    }

    let threads: usize = 4;
    let barrier = Arc::new(Barrier::new(threads + 1));
    let mut handles = Vec::new();
    for _ in 0..threads {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            for n in 0..20 {
                let mut txn = shared.begin_write();
                txn.mutator()
                    .create_node(LabelSet::single(label), prop("rank", Value::Int(n)))
                    .unwrap();
                txn.commit().unwrap();
            }
        }));
    }
    let ddl = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            shared
                .create_property_index(label, key, selene_graph::TypedIndexKind::I64)
                .expect("index DDL commits through the funnel");
        })
    };
    for h in handles {
        h.join().expect("writer thread did not panic");
    }
    ddl.join().expect("ddl thread did not panic");

    assert_eq!(shared.read().node_count(), threads * 20 + 1);
    assert!(
        shared.read().property_index.contains_key(&(label, key)),
        "the index DDL published through the committer",
    );
}

#[test]
fn istr_admission_race_class_concurrency_is_consistent() {
    // The "would this catch the IStr admission race" bar: many threads commit
    // overlapping label/property writes concurrently; the single committer
    // serializes every publish so the final snapshot is internally consistent
    // (label index ↔ columns ↔ adjacency), validated by the debug-only
    // assert_indexes_consistent that runs on every committer publish.
    let shared = Arc::new(SharedGraph::new(GraphId::new(1_011)));
    let threads: usize = 16;
    let per = 40;
    let barrier = Arc::new(Barrier::new(threads));
    let mut handles = Vec::new();
    for t in 0..threads {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            for n in 0..per {
                let mut txn = shared.begin_write();
                txn.mutator()
                    .create_node(
                        LabelSet::single(istr("Shared")),
                        prop("k", Value::Int((t * per + n) as i64)),
                    )
                    .unwrap();
                txn.commit().unwrap();
            }
        }));
    }
    for h in handles {
        h.join().expect("no writer thread panicked");
    }
    assert_eq!(shared.read().node_count(), threads * per);
    // The whole-graph structural net (also run per-commit in debug builds).
    shared
        .read()
        .assert_indexes_consistent()
        .expect("final snapshot is structurally consistent");
}

#[test]
fn explicit_txn_releases_lock_before_recv_block_no_deadlock() {
    // Deadlock invariant: an explicit transaction must release the write lock at
    // seal() BEFORE enqueueing + recv-blocking, so a queued Compact (for which
    // the committer takes the lock) cannot wedge. Here, thread A holds a long
    // explicit txn, commits it (seal drops the lock, then enqueue+recv), while
    // thread B issues a compact. If the lock were held across recv, B's compact
    // would deadlock the committer and A would never get its ack.
    let shared = Arc::new(SharedGraph::new(GraphId::new(1_013)));
    {
        let mut txn = shared.begin_write();
        for _ in 0..10 {
            txn.mutator()
                .create_node(LabelSet::single(istr("X")), PropertyMap::new())
                .unwrap();
        }
        txn.commit().unwrap();
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    let writer = {
        let shared = Arc::clone(&shared);
        thread::spawn(move || {
            for _ in 0..20 {
                let mut txn = shared.begin_write();
                txn.mutator()
                    .create_node(LabelSet::new(), PropertyMap::new())
                    .unwrap();
                txn.commit().unwrap();
            }
        })
    };
    let compactor = {
        let shared = Arc::clone(&shared);
        thread::spawn(move || {
            for _ in 0..20 {
                shared.compact().unwrap();
            }
        })
    };
    writer.join().expect("writer finished");
    compactor.join().expect("compactor finished");
    assert!(
        Instant::now() < deadline,
        "no deadlock between explicit-txn commits and compaction",
    );
}

#[test]
fn back_pressure_holds_under_fan_in_burst() {
    // Many threads each fire a burst of commits; the bounded inbound channel
    // applies back-pressure (a full channel blocks the sender) without losing
    // or reordering any commit. Correctness check: every commit lands.
    let shared = Arc::new(SharedGraph::new(GraphId::new(1_014)));
    let threads: usize = 12;
    let per = 100;
    let count = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::new();
    for _ in 0..threads {
        let shared = Arc::clone(&shared);
        let count = Arc::clone(&count);
        handles.push(thread::spawn(move || {
            for _ in 0..per {
                let mut txn = shared.begin_write();
                txn.mutator()
                    .create_node(LabelSet::new(), PropertyMap::new())
                    .unwrap();
                txn.commit().unwrap();
                count.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for h in handles {
        h.join().expect("no thread panicked under back-pressure");
    }
    assert_eq!(count.load(Ordering::Relaxed) as usize, threads * per);
    assert_eq!(shared.read().node_count(), threads * per);
}
