use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use selene_core::{
    Change, EdgeId, GraphId, HlcTimestamp, LabelSet, NodeId, Origin, PropertyMap, Value, intern,
};
use selene_persist::{
    DEFAULT_WAL_FILE_NAME, PersistError, SectionCompression, SnapshotBuilder, SnapshotConfig,
    WalConfig, WalWriter,
};

use crate::{CORE_PROVIDER_TAG, GraphError, ProviderTag, SharedGraph};

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-graph-recover-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).unwrap();
    dir
}

fn prop(name: &str, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(intern(name).unwrap(), value)]).unwrap()
}

fn write_snapshot(dir: &Path, shared: &SharedGraph, sequence: u64) -> PathBuf {
    let provider = shared
        .index_provider_by_tag(ProviderTag(CORE_PROVIDER_TAG))
        .expect("core provider is registered");
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
    builder.finalize().unwrap()
}

fn append_wal(dir: &Path, snapshot_seq: u64, changes: &[Change]) {
    let path = dir.join(DEFAULT_WAL_FILE_NAME);
    let mut writer = WalWriter::open(
        &path,
        WalConfig {
            fsync_every_n: 1,
            snapshot_seq,
        },
    )
    .unwrap();
    writer
        .append(HlcTimestamp::zero(), Origin::Local, None, changes)
        .unwrap();
    writer.flush().unwrap();
}

fn recover_err(dir: &Path) -> GraphError {
    match SharedGraph::recover(dir) {
        Ok(_) => panic!("recovery should have failed"),
        Err(error) => error,
    }
}

fn sample_shared_graph() -> SharedGraph {
    let shared = SharedGraph::builder(GraphId::new(7)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let mut ids = Vec::new();
        for index in 0..5 {
            ids.push(
                mutator
                    .create_node(
                        LabelSet::single(intern("recover.node").unwrap()),
                        prop("recover.index", Value::Int(index)),
                    )
                    .unwrap(),
            );
        }
        mutator
            .create_edge(
                intern("recover.edge").unwrap(),
                ids[0],
                ids[1],
                prop("recover.weight", Value::Int(9)),
            )
            .unwrap();
        mutator.delete_node(ids[4]).unwrap();
    }
    txn.commit().unwrap();
    shared
}

fn large_shared_graph() -> SharedGraph {
    let shared = SharedGraph::builder(GraphId::new(13)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let mut ids = Vec::with_capacity(100);
        for index in 0..100 {
            ids.push(
                mutator
                    .create_node(
                        LabelSet::single(intern("recover.large.node").unwrap()),
                        prop("recover.large.index", Value::Int(index)),
                    )
                    .unwrap(),
            );
        }
        for index in 0..200 {
            let source = ids[10 + (index % 90)];
            let target = ids[10 + ((index * 7 + 1) % 90)];
            mutator
                .create_edge(
                    intern("recover.large.edge").unwrap(),
                    source,
                    target,
                    prop("recover.large.weight", Value::Int(index as i64)),
                )
                .unwrap();
        }
        for id in ids.iter().take(10).copied() {
            mutator.delete_node(id).unwrap();
        }
    }
    txn.commit().unwrap();
    shared
}

fn node_created(id: u64) -> Change {
    Change::NodeCreated {
        id: NodeId::new(id),
        labels: LabelSet::single(intern("recover.wal.node").unwrap()),
        properties: prop("recover.id", Value::Int(id as i64)),
    }
}

#[test]
fn recover_from_empty_dir_returns_empty_graph() {
    let dir = temp_dir("empty");
    let recovered = SharedGraph::recover(&dir).unwrap();
    assert_eq!(recovered.read().node_count(), 0);
    assert_eq!(recovered.read().edge_count(), 0);
    assert!(
        recovered
            .index_provider_by_tag(ProviderTag(CORE_PROVIDER_TAG))
            .is_some()
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_from_snapshot_only_round_trips_nodes_and_edges() {
    let dir = temp_dir("snapshot");
    let shared = sample_shared_graph();
    write_snapshot(&dir, &shared, 1);

    let recovered = SharedGraph::recover(&dir).unwrap();
    let snapshot = recovered.read();
    assert_eq!(snapshot.node_count(), 4);
    assert_eq!(snapshot.edge_count(), 1);
    assert!(snapshot.is_node_alive(NodeId::new(1)));
    assert!(!snapshot.is_node_alive(NodeId::new(5)));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_from_wal_only_replays_changes_to_state() {
    let dir = temp_dir("wal-only");
    let changes = vec![
        node_created(1),
        node_created(2),
        Change::EdgeCreated {
            id: EdgeId::new(1),
            label: intern("recover.wal.edge").unwrap(),
            source: NodeId::new(1),
            target: NodeId::new(2),
            properties: PropertyMap::new(),
        },
    ];
    append_wal(&dir, 0, &changes);

    let recovered = SharedGraph::recover(&dir).unwrap();
    let snapshot = recovered.read();
    assert_eq!(snapshot.node_count(), 2);
    assert_eq!(snapshot.edge_count(), 1);
    assert!(snapshot.outgoing_edges(NodeId::new(1)).is_some());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_from_snapshot_and_wal_streams_only_post_snapshot_changes() {
    let dir = temp_dir("snapshot-wal");
    let shared = sample_shared_graph();
    write_snapshot(&dir, &shared, 3);
    append_wal(&dir, 3, &[node_created(6)]);

    let recovered = SharedGraph::recover(&dir).unwrap();
    let snapshot = recovered.read();
    assert!(snapshot.is_node_alive(NodeId::new(1)));
    assert!(snapshot.is_node_alive(NodeId::new(6)));
    assert!(!snapshot.is_node_alive(NodeId::new(5)));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_then_commit_continues_id_allocation_above_recovered_floor() {
    let dir = temp_dir("allocation");
    append_wal(&dir, 0, &[node_created(49)]);

    let recovered = SharedGraph::recover(&dir).unwrap();
    let mut txn = recovered.begin_write();
    let id = {
        let mut mutator = txn.mutator();
        mutator
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap()
    };
    assert_eq!(id, NodeId::new(50));
    txn.commit().unwrap();
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_with_corrupt_snapshot_returns_persist_error() {
    let dir = temp_dir("corrupt");
    let shared = sample_shared_graph();
    let path = write_snapshot(&dir, &shared, 1);
    let mut bytes = fs::read(&path).unwrap();
    let last = bytes.last_mut().unwrap();
    *last ^= 0xAA;
    fs::write(&path, bytes).unwrap();

    let err = recover_err(&dir);
    assert!(matches!(
        err,
        GraphError::Persist(PersistError::BodyHashMismatch { .. })
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_returns_persist_error_for_unknown_provider_in_section_table() {
    let dir = temp_dir("unknown-provider");
    let mut builder = SnapshotBuilder::new(SnapshotConfig {
        dir: dir.clone(),
        sequence: 1,
        compression: SectionCompression::None,
        fsync: false,
    });
    builder
        .add_section(*b"MISS", *b"BODY", vec![1_u8, 2, 3])
        .unwrap();
    builder.finalize().unwrap();

    let err = recover_err(&dir);
    assert!(matches!(
        err,
        GraphError::Persist(PersistError::UnknownProvider { provider, sub })
            if provider == *b"MISS" && sub == *b"BODY"
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn live_mutate_snapshot_recover_matches_state() {
    let dir = temp_dir("e2e");
    let shared = large_shared_graph();
    let expected = shared.read();
    write_snapshot(&dir, &shared, expected.meta.generation);

    let recovered = SharedGraph::recover(&dir).unwrap();
    let snapshot = recovered.read();
    assert_eq!(snapshot.node_count(), expected.node_count());
    assert_eq!(snapshot.edge_count(), expected.edge_count());
    assert_eq!(snapshot.meta.next_node_id, expected.meta.next_node_id);
    assert_eq!(snapshot.meta.next_edge_id, expected.meta.next_edge_id);
    assert!(!snapshot.is_node_alive(NodeId::new(1)));
    assert!(!snapshot.is_node_alive(NodeId::new(10)));
    assert!(snapshot.is_node_alive(NodeId::new(11)));
    assert!(snapshot.is_edge_alive(EdgeId::new(1)));
    assert_eq!(
        snapshot.edge_endpoints(EdgeId::new(1)),
        expected.edge_endpoints(EdgeId::new(1))
    );
    let _ = fs::remove_dir_all(dir);
}
