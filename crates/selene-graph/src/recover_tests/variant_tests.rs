use std::fs;

use selene_core::{
    Change, EdgeId, EdgeTypeDefV1, GraphId, GraphTypeId, LabelDiff, LabelSet, NodeId, NodeTypeRef,
    PropertyDiff, PropertyMap, SchemaChange, Value, intern,
};
use smallvec::smallvec;

use crate::{
    DropBehavior, EdgeEndpointDef, NodeTypeDef, PropertyTypeDef, SharedGraph, TypedIndexKind,
    ValidationMode,
};

use super::{append_wal, expect_prop, prop, temp_dir};

fn person_closed_graph_type() -> crate::GraphTypeDef {
    let person = intern("recover.closed.person").unwrap();
    crate::GraphTypeDef {
        name: intern("recover.closed.person.graph").unwrap(),
        node_types: vec![NodeTypeDef {
            name: person,
            key_labels: LabelSet::single(person),
            properties: Vec::new(),
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    }
}

fn colliding_legacy_endpoint_graph_type() -> crate::GraphTypeDef {
    let legacy_label = intern("recover.legacy.person").unwrap();
    crate::GraphTypeDef {
        name: intern("recover.legacy.collision.graph").unwrap(),
        node_types: vec![
            NodeTypeDef {
                name: intern("recover.legacy.person.type").unwrap(),
                key_labels: LabelSet::single(legacy_label),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: legacy_label,
                key_labels: LabelSet::single(intern("recover.legacy.other").unwrap()),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
        ],
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
fn recover_from_wal_only_replays_removed_variants() {
    let dir = temp_dir("removed-variants");
    let graph_id = GraphId::new(703);
    let shared = SharedGraph::new(graph_id);
    let base = intern("recover.remove.base").unwrap();
    let removed_label = intern("recover.remove.label").unwrap();
    let node_prop = intern("recover.remove.node_prop").unwrap();
    let edge_prop = intern("recover.remove.edge_prop").unwrap();
    let edge_label = intern("recover.remove.edge").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let left = mutator
            .create_node(
                LabelSet::from_iter([base, removed_label]),
                PropertyMap::from_pairs([(node_prop, Value::Int(1))]).unwrap(),
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
                PropertyMap::from_pairs([(edge_prop, Value::Int(2))]).unwrap(),
            )
            .unwrap();
        mutator.remove_node_property(left, node_prop).unwrap();
        mutator.remove_node_label(left, removed_label).unwrap();
        mutator.remove_edge_property(edge, edge_prop).unwrap();
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
            .create_edge_type(
                rel,
                rel,
                EdgeEndpointDef::NodeType(0),
                EdgeEndpointDef::NodeType(0),
                Vec::<PropertyTypeDef>::new(),
                ValidationMode::Strict,
            )
            .unwrap();
        mutator.drop_edge_type(rel, DropBehavior::Restrict).unwrap();
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
                change: selene_core::SchemaChange::EdgeTypeAddedV2 { .. },
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
fn recover_closed_legacy_edge_endpoint_prefers_label_set_over_type_name() {
    let dir = temp_dir("legacy-edge-endpoint-label-precedence");
    let graph_id = GraphId::new(708);
    let base = colliding_legacy_endpoint_graph_type();
    let legacy_label = base.node_types[0]
        .key_labels
        .iter()
        .next()
        .copied()
        .unwrap();
    let rel = intern("recover.legacy.knows").unwrap();
    append_wal(
        &dir,
        0,
        &[Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::EdgeTypeAdded {
                graph_type: GraphTypeId::new(1).unwrap(),
                label: rel,
                def: EdgeTypeDefV1 {
                    label: rel,
                    source_node_type: NodeTypeRef(legacy_label),
                    target_node_type: NodeTypeRef(legacy_label),
                    properties: smallvec![],
                },
            },
        }],
    );

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    assert_eq!(
        graph_type.edge_types[0].source_node_type,
        EdgeEndpointDef::NodeType(0)
    );
    assert_eq!(
        graph_type.edge_types[0].target_node_type,
        EdgeEndpointDef::NodeType(0)
    );
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
        txn.mutator()
            .drop_node_type(person, DropBehavior::Restrict)
            .unwrap();
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
fn recover_from_wal_only_replays_cascade_truncate_then_node_type_dropped() {
    // CASCADE drop emits [NodesOfTypeTruncated, NodeTypeDropped] in one txn;
    // recovery must replay both in WAL order to the identical post-state:
    // instances gone (re-derived from recovered store state) AND type dropped.
    let dir = temp_dir("cascade-node-drop-replay");
    let graph_id = GraphId::new(709);
    let base = person_closed_graph_type();
    let person = base.node_types[0].name;
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let create_outcome = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(person), PropertyMap::new())
            .unwrap();
        txn.mutator()
            .create_node(LabelSet::single(person), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap()
    };
    assert_eq!(shared.read().node_count(), 2);

    let cascade_outcome = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .drop_node_type(person, DropBehavior::Cascade)
            .unwrap();
        txn.commit().unwrap()
    };
    // The CASCADE changeset is exactly truncate-then-drop, in order.
    assert!(matches!(
        cascade_outcome.changes.as_slice(),
        [
            Change::NodesOfTypeTruncated { .. },
            Change::SchemaChanged {
                change: selene_core::SchemaChange::NodeTypeDropped { .. },
                ..
            }
        ]
    ));
    // Replay all changes in WAL order: the two node creations, then the
    // CASCADE truncate-then-drop. The truncate re-derives the live rows it
    // removes from the recovered store state (no ids persisted).
    let mut all_changes = create_outcome.changes.clone();
    all_changes.extend(cascade_outcome.changes.iter().cloned());
    append_wal(&dir, 0, &all_changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    assert!(graph_type.node_types.is_empty());
    assert!(graph_type.edge_types.is_empty());
    assert_eq!(recovered.read().node_count(), 0);
    assert_eq!(recovered.read().edge_count(), 0);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_from_wal_only_replays_graph_reset_to_empty_and_open() {
    // BRIEF-152: DROP GRAPH emits one declarative Change::GraphReset. Recovery
    // must replay it by re-deriving every live row from the recovered store and
    // marking it dead (no ids persisted), and must reset the schema to open —
    // even when recover_closed is handed a bound type, the replayed reset wins,
    // reconstructing the identical empty+open post-state the runtime produced.
    let dir = temp_dir("graph-reset-replay");
    let graph_id = GraphId::new(710);
    let base = person_closed_graph_type();
    let person = base.node_types[0].name;
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let create_outcome = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(person), PropertyMap::new())
            .unwrap();
        txn.mutator()
            .create_node(LabelSet::single(person), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap()
    };
    assert_eq!(shared.read().node_count(), 2);

    let reset_outcome = {
        let mut txn = shared.begin_write();
        txn.mutator().factory_reset().unwrap();
        txn.commit().unwrap()
    };
    // O(1): the persisted changeset is exactly one declarative GraphReset.
    assert!(matches!(
        reset_outcome.changes.as_slice(),
        [Change::GraphReset {}]
    ));

    // Replay create...+GraphReset in WAL order. recover_closed is GIVEN the
    // closed `base`, but the replayed reset forces the recovered graph open.
    let mut all_changes = create_outcome.changes.clone();
    all_changes.extend(reset_outcome.changes.iter().cloned());
    append_wal(&dir, 0, &all_changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    assert!(
        recovered.graph_type().is_none(),
        "GraphReset replay resets the schema to open, overriding the recover_closed base"
    );
    assert!(!recovered.is_closed());
    assert_eq!(
        recovered.read().node_count(),
        0,
        "all nodes wiped on replay"
    );
    assert_eq!(
        recovered.read().edge_count(),
        0,
        "all edges wiped on replay"
    );
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
                change: selene_core::SchemaChange::PropertyIndexCreatedNamed { .. },
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
                change: selene_core::SchemaChange::PropertyIndexCreatedNamed { .. },
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
