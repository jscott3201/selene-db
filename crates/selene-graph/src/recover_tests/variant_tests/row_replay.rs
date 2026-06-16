use std::fs;

use selene_core::{
    Change, EdgeId, GraphId, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap, Value,
    db_string,
};

use crate::SharedGraph;

use super::super::{append_wal, expect_prop, prop, temp_dir};

#[test]
fn recover_from_wal_only_replays_node_updated() {
    let dir = temp_dir("node-updated");
    let shared = SharedGraph::new(GraphId::new(701));
    let base = db_string("recover.node.base").unwrap();
    let added = db_string("recover.node.added").unwrap();
    let name = db_string("recover.node.name").unwrap();
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
                LabelDiff::new([added.clone()], []).unwrap(),
                PropertyDiff::new(
                    [
                        (db_string("recover.node.age").unwrap(), Value::Int(31)),
                        (name, Value::String(db_string("Alice").unwrap())),
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
                LabelSet::single(db_string("recover.edge.left").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        let right = mutator
            .create_node(
                LabelSet::single(db_string("recover.edge.right").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        let edge = mutator
            .create_edge(
                db_string("recover.edge.rel").unwrap(),
                left,
                right,
                prop("recover.edge.weight", Value::Int(1)),
            )
            .unwrap();
        mutator
            .update_edge(
                edge,
                PropertyDiff::new(
                    [(db_string("recover.edge.weight").unwrap(), Value::Int(9))],
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
fn recover_from_wal_only_replays_removed_variants() {
    let dir = temp_dir("removed-variants");
    let graph_id = GraphId::new(703);
    let shared = SharedGraph::new(graph_id);
    let base = db_string("recover.remove.base").unwrap();
    let removed_label = db_string("recover.remove.label").unwrap();
    let node_prop = db_string("recover.remove.node_prop").unwrap();
    let edge_prop = db_string("recover.remove.edge_prop").unwrap();
    let edge_label = db_string("recover.remove.edge").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let left = mutator
            .create_node(
                LabelSet::from_iter([base.clone(), removed_label.clone()]),
                PropertyMap::from_pairs([(node_prop.clone(), Value::Int(1))]).unwrap(),
            )
            .unwrap();
        let right = mutator
            .create_node(LabelSet::single(base), PropertyMap::new())
            .unwrap();
        let edge = mutator
            .create_edge(
                edge_label,
                left,
                right,
                PropertyMap::from_pairs([(edge_prop.clone(), Value::Int(2))]).unwrap(),
            )
            .unwrap();
        mutator
            .remove_node_property(left, node_prop.clone())
            .unwrap();
        mutator
            .remove_node_label(left, removed_label.clone())
            .unwrap();
        mutator
            .remove_edge_property(edge, edge_prop.clone())
            .unwrap();
        txn.commit().unwrap()
    };
    append_wal(&dir, 0, &outcome.changes);

    let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
    let snapshot = recovered.read();
    assert!(
        snapshot
            .node_properties(NodeId::new(1))
            .unwrap()
            .get(&node_prop)
            .is_none()
    );
    assert!(
        !snapshot
            .node_labels(NodeId::new(1))
            .unwrap()
            .contains(&removed_label)
    );
    assert!(
        snapshot
            .edge_properties(EdgeId::new(1))
            .unwrap()
            .get(&edge_prop)
            .is_none()
    );
    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::NodeCreated { .. },
            Change::NodeCreated { .. },
            Change::EdgeCreated { .. },
            Change::NodePropertyRemoved { .. },
            Change::NodeLabelRemoved { .. },
            Change::EdgePropertyRemoved { .. }
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
                LabelSet::single(db_string("recover.delete.left").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        let right = mutator
            .create_node(
                LabelSet::single(db_string("recover.delete.right").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        let edge = mutator
            .create_edge(
                db_string("recover.delete.rel").unwrap(),
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
