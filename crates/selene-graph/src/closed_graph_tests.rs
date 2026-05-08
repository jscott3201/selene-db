use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use selene_core::{
    Change, EdgeId, GraphId, HlcTimestamp, LabelSet, NodeId, Origin, PropertyMap,
    PropertyValueType, Value, intern,
};
use selene_persist::{
    DEFAULT_WAL_FILE_NAME, SectionCompression, SnapshotBuilder, SnapshotConfig, WalConfig,
    WalWriter,
};

use crate::{
    CORE_PROVIDER_TAG, EntityId, GraphError, GraphTypeDef, NodeTypeDef, PropertyTypeDef,
    ProviderTag, SharedGraph, TypeViolation,
};

fn istr(name: &str) -> selene_core::IStr {
    intern(name).unwrap()
}

fn prop(name: &str, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(istr(name), value)]).unwrap()
}

fn person_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: istr("closed.person.graph"),
        node_types: vec![NodeTypeDef {
            name: istr("closed.person"),
            key_labels: LabelSet::single(istr("Person")),
            properties: vec![PropertyTypeDef {
                name: istr("name"),
                value_type: PropertyValueType::String,
                required: true,
            }],
        }],
        edge_types: vec![crate::EdgeTypeDef {
            name: istr("closed.knows"),
            label: istr("KNOWS"),
            source_node_type: 0,
            target_node_type: 0,
            properties: vec![PropertyTypeDef {
                name: istr("since"),
                value_type: PropertyValueType::Int,
                required: false,
            }],
        }],
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-closed-graph-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).unwrap();
    dir
}

fn write_snapshot(dir: &Path, shared: &SharedGraph, sequence: u64) {
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
    builder.finalize().unwrap();
}

fn append_wal(dir: &Path, snapshot_seq: u64, changes: &[Change]) {
    let mut writer = WalWriter::open(
        &dir.join(DEFAULT_WAL_FILE_NAME),
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

#[test]
fn open_graph_commits_unchanged() {
    let shared = SharedGraph::builder(GraphId::new(1)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(LabelSet::single(istr("Anything")), PropertyMap::new())
            .unwrap();
    }
    txn.commit().unwrap();
    assert!(!shared.is_closed());
    assert_eq!(shared.read().node_count(), 1);
}

#[test]
fn closed_graph_accepts_valid_commit() {
    let shared = SharedGraph::builder(GraphId::new(2))
        .bound_to(person_graph_type())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    let id = {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::String(istr("Alice"))),
            )
            .unwrap()
    };
    txn.commit().unwrap();
    assert!(shared.read().is_node_alive(id));
}

#[test]
fn closed_graph_rejects_invalid_commit_without_publishing() {
    let shared = SharedGraph::builder(GraphId::new(3))
        .bound_to(person_graph_type())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        assert_eq!(
            mutator
                .create_node(LabelSet::single(istr("Person")), PropertyMap::new())
                .unwrap(),
            NodeId::new(1)
        );
    }
    let err = txn.commit().unwrap_err();
    assert!(matches!(
        err,
        GraphError::TypeViolation(TypeViolation::MissingRequiredProperty {
            entity_id,
            property,
            ..
        }) if entity_id == EntityId::Node(NodeId::new(1)) && property == istr("name")
    ));
    assert_eq!(shared.read().node_count(), 0);

    let mut txn = shared.begin_write();
    let id = {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::String(istr("Bob"))),
            )
            .unwrap()
    };
    assert_eq!(id, NodeId::new(2), "D11 allocator hole is preserved");
    txn.commit().unwrap();
}

#[test]
fn closed_graph_rejects_edge_endpoint_mismatch() {
    let shared = SharedGraph::builder(GraphId::new(4))
        .bound_to(person_graph_type())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let alice = mutator
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::String(istr("Alice"))),
            )
            .unwrap();
        let project = mutator
            .create_node(
                LabelSet::single(istr("Project")),
                prop("name", Value::String(istr("Apollo"))),
            )
            .unwrap();
        mutator
            .create_edge(istr("KNOWS"), alice, project, PropertyMap::new())
            .unwrap();
    }
    assert!(matches!(
        txn.commit().unwrap_err(),
        GraphError::TypeViolation(TypeViolation::UnknownNodeLabel {
            id,
            ..
        }) if id == NodeId::new(2)
    ));
}

#[test]
fn recover_round_trips_bound_graph_type_and_rearms_validator() {
    let dir = temp_dir("roundtrip");
    let graph_type = person_graph_type();
    let shared = SharedGraph::builder(GraphId::new(5))
        .bound_to(graph_type.clone())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let alice = mutator
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::String(istr("Alice"))),
            )
            .unwrap();
        let bob = mutator
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::String(istr("Bob"))),
            )
            .unwrap();
        mutator
            .create_edge(istr("KNOWS"), alice, bob, prop("since", Value::Int(2026)))
            .unwrap();
    }
    txn.commit().unwrap();
    let sequence = shared.read().meta.generation;
    write_snapshot(&dir, &shared, sequence);
    append_wal(
        &dir,
        sequence,
        &[Change::NodeCreated {
            id: NodeId::new(3),
            labels: LabelSet::single(istr("Person")),
            properties: prop("name", Value::String(istr("Carol"))),
        }],
    );

    let recovered = SharedGraph::recover(&dir, GraphId::new(5)).unwrap();
    assert!(recovered.is_closed());
    assert_eq!(recovered.graph_type().as_deref(), Some(&graph_type));
    assert!(recovered.read().is_node_alive(NodeId::new(3)));

    let mut txn = recovered.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_edge(
                istr("KNOWS"),
                NodeId::new(1),
                NodeId::new(2),
                prop("since", Value::String(istr("bad"))),
            )
            .unwrap();
    }
    assert!(matches!(
        txn.commit().unwrap_err(),
        GraphError::TypeViolation(TypeViolation::PropertyTypeMismatch {
            entity_id,
            property,
            expected: PropertyValueType::Int,
            observed: "String",
        }) if entity_id == EntityId::Edge(EdgeId::new(2)) && property == istr("since")
    ));
    let _ = fs::remove_dir_all(dir);
}
