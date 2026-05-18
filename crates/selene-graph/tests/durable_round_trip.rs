//! Durable WAL integration tests.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use selene_core::{GraphId, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_graph::SharedGraph;
use selene_persist::{DEFAULT_WAL_FILE_NAME, WalConfig};

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-graph-durable-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).unwrap();
    dir
}

fn prop(name: &str, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(intern(name).unwrap(), value)]).unwrap()
}

#[test]
fn durable_round_trip_recovery() {
    let dir = temp_dir("round-trip");
    let wal_path = dir.join(DEFAULT_WAL_FILE_NAME);
    let graph_id = GraphId::new(77);
    let label = intern("durable.node").unwrap();
    let key = intern("durable.name").unwrap();
    let value = intern("written").unwrap();

    {
        let shared = SharedGraph::builder(graph_id)
            .with_wal(&wal_path, WalConfig::default())
            .unwrap()
            .build()
            .unwrap();
        let mut txn = shared.begin_write();
        let node_id = {
            let mut mutator = txn.mutator();
            mutator
                .create_node(
                    LabelSet::single(label),
                    prop("durable.name", Value::String(value)),
                )
                .unwrap()
        };
        let outcome = txn.commit().unwrap();
        assert_eq!(node_id, NodeId::new(1));
        assert_eq!(outcome.durable_at, Some(1));
    }

    let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
    let snapshot = recovered.read();
    assert_eq!(snapshot.node_count(), 1);
    assert_eq!(
        snapshot
            .node_store
            .properties
            .get(0)
            .and_then(|props| props.get(&key)),
        Some(&Value::String(value))
    );
}

#[test]
fn post_recovery_commits_remain_durable() {
    let dir = temp_dir("post-recovery");
    let graph_id = GraphId::new(78);
    let label = intern("durable.recover.node").unwrap();

    {
        let shared = SharedGraph::builder(graph_id)
            .with_wal(dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default())
            .unwrap()
            .build()
            .unwrap();
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(label), PropertyMap::new())
            .unwrap();
        let outcome = txn.commit().unwrap();
        assert_eq!(outcome.durable_at, Some(1));
    }

    let after_first_recover = SharedGraph::recover(&dir, graph_id).unwrap();
    assert_eq!(after_first_recover.read().node_count(), 1);

    {
        let mut txn = after_first_recover.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(label), PropertyMap::new())
            .unwrap();
        let outcome = txn.commit().unwrap();
        // Post-recovery durability proof: the recovered graph reopened the
        // WAL and the second commit advanced the durable sequence past the
        // recovered tip.
        assert_eq!(outcome.durable_at, Some(2));
    }
    drop(after_first_recover);

    let after_second_recover = SharedGraph::recover(&dir, graph_id).unwrap();
    assert_eq!(after_second_recover.read().node_count(), 2);
}
