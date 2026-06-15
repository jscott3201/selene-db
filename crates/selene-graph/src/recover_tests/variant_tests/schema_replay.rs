use std::fs;

use selene_core::{
    Change, EdgeTypeDefV1, GraphId, GraphTypeId, LabelSet, NodeTypeRef, PropertyMap, SchemaChange,
    Value, db_string,
};
use smallvec::smallvec;

use crate::{
    DropBehavior, EdgeEndpointDef, PropertyTypeDef, SharedGraph, TypedIndexKind, ValidationMode,
};

use super::super::{append_wal, temp_dir};
use super::{colliding_legacy_endpoint_graph_type, person_closed_graph_type};

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
    let rel = db_string("recover.closed.knows").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_edge_type(
                rel.clone(),
                rel.clone(),
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
        .cloned()
        .unwrap();
    let rel = db_string("recover.legacy.knows").unwrap();
    append_wal(
        &dir,
        0,
        &[Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::EdgeTypeAdded {
                graph_type: GraphTypeId::new(1).unwrap(),
                label: rel.clone(),
                def: EdgeTypeDefV1 {
                    label: rel,
                    source_node_type: NodeTypeRef(legacy_label.clone()),
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
    let person = base.node_types[0].name.clone();
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
    let person = base.node_types[0].name.clone();
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let create_outcome = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(person.clone()), PropertyMap::new())
            .unwrap();
        txn.mutator()
            .create_node(LabelSet::single(person.clone()), PropertyMap::new())
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
    let person = base.node_types[0].name.clone();
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let create_outcome = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(person.clone()), PropertyMap::new())
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
    let label = db_string("recover.index.created.label").unwrap();
    let property = db_string("recover.index.created.occurred_at").unwrap();
    let timestamp = Value::ZonedDateTime(Box::new(
        "2026-05-07T12:34:56-04:00[America/New_York]"
            .parse()
            .unwrap(),
    ));
    let outcome = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(label.clone()),
                PropertyMap::from_pairs([(property.clone(), timestamp.clone())]).unwrap(),
            )
            .unwrap();
        mutator
            .create_property_index(
                label.clone(),
                property.clone(),
                TypedIndexKind::ZonedDateTime,
            )
            .unwrap();
        txn.commit().unwrap()
    };
    append_wal(&dir, 0, &outcome.changes);

    let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
    let snapshot = recovered.read();
    assert_eq!(snapshot.property_index_count(), 1);
    assert!(snapshot.property_index_for(&label, &property).is_some());
    let rows = snapshot
        .nodes_with_property_eq(&label, &property, &timestamp)
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
    let label = db_string("recover.index.dropped.label").unwrap();
    let property = db_string("recover.index.dropped.age").unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(label.clone()),
                PropertyMap::from_pairs([(property.clone(), Value::Int(42))]).unwrap(),
            )
            .unwrap();
        mutator
            .create_property_index(label.clone(), property.clone(), TypedIndexKind::I64)
            .unwrap();
        mutator
            .drop_property_index(label.clone(), property.clone())
            .unwrap();
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
