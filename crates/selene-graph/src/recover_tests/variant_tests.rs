use std::fs;

use selene_core::{
    Change, EdgeId, GraphId, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap, Value, intern,
};

use crate::{NodeTypeDef, PropertyTypeDef, SharedGraph, TypedIndexKind};

use super::{append_wal, expect_prop, prop, temp_dir};

fn person_closed_graph_type() -> crate::GraphTypeDef {
    let person = intern("recover.closed.person").unwrap();
    crate::GraphTypeDef {
        name: intern("recover.closed.person.graph").unwrap(),
        node_types: vec![NodeTypeDef {
            name: person,
            key_labels: LabelSet::single(person),
            properties: Vec::new(),
        }],
        edge_types: Vec::new(),
    }
}

#[test]
fn recover_from_wal_only_replays_node_updated() {
    let dir = temp_dir("node-updated");
    let shared = SharedGraph::new(GraphId::new(701));
    let base = intern("recover.node.base").unwrap();
    let added = intern("recover.node.added").unwrap();
    let name = intern("recover.node.name").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let node = mutator
            .create_node(
                LabelSet::single(base),
                prop("recover.node.age", Value::Int(30)),
            )
            .unwrap();
        mutator
            .update_node(
                node,
                LabelDiff::new([added], []).unwrap(),
                PropertyDiff::new(
                    [
                        (intern("recover.node.age").unwrap(), Value::Int(31)),
                        (name, Value::String(intern("Alice").unwrap())),
                    ],
                    [],
                )
                .unwrap(),
            )
            .unwrap();
        txn.commit().unwrap()
    };
    let expected = shared.read();
    append_wal(&dir, 0, &outcome.changes);

    let recovered = SharedGraph::recover(&dir, GraphId::new(701)).unwrap();
    let snapshot = recovered.read();
    assert_eq!(
        snapshot.node_labels(NodeId::new(1)),
        expected.node_labels(NodeId::new(1))
    );
    assert!(
        snapshot
            .node_labels(NodeId::new(1))
            .unwrap()
            .contains(&added)
    );
    assert_eq!(
        snapshot.node_properties(NodeId::new(1)),
        expected.node_properties(NodeId::new(1))
    );
    expect_prop(
        snapshot.node_properties(NodeId::new(1)).unwrap(),
        "recover.node.age",
        &Value::Int(31),
    );
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::NodeCreated { .. }, Change::NodeUpdated { .. }]
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_from_wal_only_replays_edge_updated() {
    let dir = temp_dir("edge-updated");
    let shared = SharedGraph::new(GraphId::new(702));
    let outcome = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let left = mutator
            .create_node(
                LabelSet::single(intern("recover.edge.left").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        let right = mutator
            .create_node(
                LabelSet::single(intern("recover.edge.right").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        let edge = mutator
            .create_edge(
                intern("recover.edge.rel").unwrap(),
                left,
                right,
                prop("recover.edge.weight", Value::Int(1)),
            )
            .unwrap();
        mutator
            .update_edge(
                edge,
                PropertyDiff::new(
                    [(intern("recover.edge.weight").unwrap(), Value::Int(9))],
                    [],
                )
                .unwrap(),
            )
            .unwrap();
        txn.commit().unwrap()
    };
    let expected = shared.read();
    append_wal(&dir, 0, &outcome.changes);

    let recovered = SharedGraph::recover(&dir, GraphId::new(702)).unwrap();
    let snapshot = recovered.read();
    assert_eq!(snapshot.edge_count(), expected.edge_count());
    assert_eq!(
        snapshot.edge_endpoints(EdgeId::new(1)),
        expected.edge_endpoints(EdgeId::new(1))
    );
    assert_eq!(
        snapshot.edge_properties(EdgeId::new(1)),
        expected.edge_properties(EdgeId::new(1))
    );
    expect_prop(
        snapshot.edge_properties(EdgeId::new(1)).unwrap(),
        "recover.edge.weight",
        &Value::Int(9),
    );
    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::NodeCreated { .. },
            Change::NodeCreated { .. },
            Change::EdgeCreated { .. },
            Change::EdgeUpdated { .. }
        ]
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_from_wal_only_replays_edge_deleted() {
    let dir = temp_dir("edge-deleted");
    let shared = SharedGraph::new(GraphId::new(703));
    let outcome = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let left = mutator
            .create_node(
                LabelSet::single(intern("recover.delete.left").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        let right = mutator
            .create_node(
                LabelSet::single(intern("recover.delete.right").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        let edge = mutator
            .create_edge(
                intern("recover.delete.rel").unwrap(),
                left,
                right,
                PropertyMap::new(),
            )
            .unwrap();
        mutator.delete_edge(edge).unwrap();
        txn.commit().unwrap()
    };
    append_wal(&dir, 0, &outcome.changes);

    let recovered = SharedGraph::recover(&dir, GraphId::new(703)).unwrap();
    let snapshot = recovered.read();
    assert_eq!(snapshot.node_count(), 2);
    assert_eq!(snapshot.edge_count(), 0);
    assert!(!snapshot.is_edge_alive(EdgeId::new(1)));
    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::NodeCreated { .. },
            Change::NodeCreated { .. },
            Change::EdgeCreated { .. },
            Change::EdgeDeleted { .. }
        ]
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_from_wal_only_replays_edge_type_added_and_dropped() {
    let dir = temp_dir("edge-type-add-drop");
    let graph_id = GraphId::new(704);
    let base = person_closed_graph_type();
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let rel = intern("recover.closed.knows").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_edge_type(rel, rel, 0, 0, Vec::<PropertyTypeDef>::new())
            .unwrap();
        mutator.drop_edge_type(rel).unwrap();
        txn.commit().unwrap()
    };
    append_wal(&dir, 0, &outcome.changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    assert_eq!(graph_type.node_types.len(), 1);
    assert!(graph_type.edge_types.is_empty());
    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::SchemaChanged {
                change: selene_core::SchemaChange::EdgeTypeAdded { .. },
                ..
            },
            Change::SchemaChanged {
                change: selene_core::SchemaChange::EdgeTypeDropped { .. },
                ..
            }
        ]
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_from_wal_only_replays_node_type_dropped() {
    let dir = temp_dir("node-type-dropped");
    let graph_id = GraphId::new(705);
    let base = person_closed_graph_type();
    let person = base.node_types[0].name;
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        txn.mutator().drop_node_type(person).unwrap();
        txn.commit().unwrap()
    };
    append_wal(&dir, 0, &outcome.changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    assert!(graph_type.node_types.is_empty());
    assert!(graph_type.edge_types.is_empty());
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::SchemaChanged {
            change: selene_core::SchemaChange::NodeTypeDropped { .. },
            ..
        }]
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_from_wal_only_replays_property_index_created() {
    let dir = temp_dir("property-index-created");
    let graph_id = GraphId::new(706);
    let shared = SharedGraph::new(graph_id);
    let label = intern("recover.index.created.label").unwrap();
    let property = intern("recover.index.created.age").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(label),
                PropertyMap::from_pairs([(property, Value::Int(42))]).unwrap(),
            )
            .unwrap();
        mutator
            .create_property_index(label, property, TypedIndexKind::I64)
            .unwrap();
        txn.commit().unwrap()
    };
    append_wal(&dir, 0, &outcome.changes);

    let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
    let snapshot = recovered.read();
    assert_eq!(snapshot.property_index_count(), 1);
    assert!(snapshot.property_index_for(&label, &property).is_some());
    let rows = snapshot
        .nodes_with_property_eq(&label, &property, &Value::Int(42))
        .unwrap();
    assert_eq!(rows.iter().collect::<Vec<_>>(), vec![0]);
    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::NodeCreated { .. },
            Change::SchemaChanged {
                change: selene_core::SchemaChange::PropertyIndexCreated { .. },
                ..
            }
        ]
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_from_wal_only_replays_property_index_dropped() {
    let dir = temp_dir("property-index-dropped");
    let graph_id = GraphId::new(707);
    let shared = SharedGraph::new(graph_id);
    let label = intern("recover.index.dropped.label").unwrap();
    let property = intern("recover.index.dropped.age").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(label),
                PropertyMap::from_pairs([(property, Value::Int(42))]).unwrap(),
            )
            .unwrap();
        mutator
            .create_property_index(label, property, TypedIndexKind::I64)
            .unwrap();
        mutator.drop_property_index(label, property).unwrap();
        txn.commit().unwrap()
    };
    append_wal(&dir, 0, &outcome.changes);

    let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
    let snapshot = recovered.read();
    assert_eq!(snapshot.node_count(), 1);
    assert_eq!(snapshot.property_index_count(), 0);
    assert!(snapshot.property_index_for(&label, &property).is_none());
    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::NodeCreated { .. },
            Change::SchemaChanged {
                change: selene_core::SchemaChange::PropertyIndexCreated { .. },
                ..
            },
            Change::SchemaChanged {
                change: selene_core::SchemaChange::PropertyIndexDropped { .. },
                ..
            }
        ]
    ));
    let _ = fs::remove_dir_all(dir);
}
