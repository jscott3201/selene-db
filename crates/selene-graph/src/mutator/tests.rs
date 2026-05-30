use proptest::prelude::*;
use roaring::RoaringBitmap;
use selene_core::{
    Change, GraphId, PredefinedValueType, PropertyValueType, Value, ValueType, intern,
};

use super::*;
use crate::SharedGraph;
use crate::graph_types::{NodeTypeDef, ValidationMode};

fn empty_node(mutator: &mut Mutator<'_, '_>) -> NodeId {
    mutator
        .create_node(LabelSet::new(), PropertyMap::new())
        .expect("create_node ok")
}

#[test]
fn create_node_returns_id_and_emits_change() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let id = {
        let mut mutator = txn.mutator();
        mutator
            .create_node(LabelSet::new(), PropertyMap::new())
            .expect("create_node ok")
    };
    let outcome = txn.commit().unwrap();
    assert_eq!(id, NodeId::new(1));
    assert!(matches!(outcome.changes[0], Change::NodeCreated { id, .. } if id == NodeId::new(1)));
}

#[test]
fn create_node_with_two_labels_indexes_both() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (a, b, id) = {
        let mut mutator = txn.mutator();
        let a = intern("node.index.a").unwrap();
        let b = intern("node.index.b").unwrap();
        let id = mutator
            .create_node(LabelSet::from_iter([a, b]), PropertyMap::new())
            .expect("create_node ok");
        assert!(mutator.read().nodes_with_label(&a).unwrap().contains(0));
        assert!(mutator.read().nodes_with_label(&b).unwrap().contains(0));
        (a, b, id)
    };
    txn.commit().unwrap();
    let snapshot = shared.read();
    assert_eq!(id, NodeId::new(1));
    assert!(snapshot.nodes_with_label(&a).unwrap().contains(0));
    assert!(snapshot.nodes_with_label(&b).unwrap().contains(0));
}

#[test]
fn update_node_label_diff_updates_indexes_atomically() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (old, new, id) = {
        let mut mutator = txn.mutator();
        let old = intern("node.index.old").unwrap();
        let new = intern("node.index.new").unwrap();
        let id = mutator
            .create_node(LabelSet::single(old), PropertyMap::new())
            .expect("create_node ok");
        mutator
            .update_node(
                id,
                LabelDiff::new([new], [old]).unwrap(),
                PropertyDiff::new([], []).unwrap(),
            )
            .unwrap();
        assert!(mutator.read().nodes_with_label(&old).is_none());
        assert!(mutator.read().nodes_with_label(&new).unwrap().contains(0));
        (old, new, id)
    };
    txn.commit().unwrap();
    let snapshot = shared.read();
    assert_eq!(id, NodeId::new(1));
    assert!(snapshot.nodes_with_label(&old).is_none());
    assert!(snapshot.nodes_with_label(&new).unwrap().contains(0));
}

#[test]
fn delete_node_removes_from_all_label_indexes() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (a, b) = {
        let mut mutator = txn.mutator();
        let a = intern("node.index.delete.a").unwrap();
        let b = intern("node.index.delete.b").unwrap();
        let id = mutator
            .create_node(LabelSet::from_iter([a, b]), PropertyMap::new())
            .expect("create_node ok");
        mutator.delete_node(id).unwrap();
        assert!(mutator.read().nodes_with_label(&a).is_none());
        assert!(mutator.read().nodes_with_label(&b).is_none());
        (a, b)
    };
    txn.commit().unwrap();
    assert!(shared.read().nodes_with_label(&a).is_none());
    assert!(shared.read().nodes_with_label(&b).is_none());
}

#[test]
fn create_edge_with_invalid_source_fails() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let target = empty_node(&mut mutator);
    let err = mutator
        .create_edge(
            intern("edge.invalid.source").unwrap(),
            NodeId::new(99),
            target,
            PropertyMap::new(),
        )
        .unwrap_err();
    assert!(matches!(err, GraphError::NodeNotFound { id } if id == NodeId::new(99)));
}

#[test]
fn create_edge_with_invalid_target_fails() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let source = empty_node(&mut mutator);
    let err = mutator
        .create_edge(
            intern("edge.invalid.target").unwrap(),
            source,
            NodeId::new(99),
            PropertyMap::new(),
        )
        .unwrap_err();
    assert!(matches!(err, GraphError::NodeNotFound { id } if id == NodeId::new(99)));
}

#[test]
fn update_node_with_unknown_id_fails() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let err = mutator
        .update_node(
            NodeId::new(1),
            LabelDiff::new([], []).unwrap(),
            PropertyDiff::new([], []).unwrap(),
        )
        .unwrap_err();
    assert!(matches!(err, GraphError::NodeNotFound { .. }));
}

#[test]
fn delete_node_cascades_to_incident_edges() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (a, b, edge) = {
        let mut mutator = txn.mutator();
        let a = empty_node(&mut mutator);
        let b = empty_node(&mut mutator);
        let edge = mutator
            .create_edge(intern("edge.cascade").unwrap(), a, b, PropertyMap::new())
            .unwrap();
        mutator.delete_node(a).unwrap();
        (a, b, edge)
    };
    txn.commit().unwrap();
    let snapshot = shared.read();
    assert!(!snapshot.is_node_alive(a));
    assert!(snapshot.is_node_alive(b));
    assert!(!snapshot.is_edge_alive(edge));
    assert!(snapshot.incoming_edges(b).is_none());
}

#[test]
fn delete_node_cascade_clears_edge_label_index() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let label = intern("edge.index.cascade").unwrap();
    {
        let mut mutator = txn.mutator();
        let a = empty_node(&mut mutator);
        let b = empty_node(&mut mutator);
        mutator
            .create_edge(label, a, b, PropertyMap::new())
            .expect("create_edge ok");
        assert!(mutator.read().edges_with_label(&label).unwrap().contains(0));
        mutator.delete_node(a).unwrap();
        assert!(mutator.read().edges_with_label(&label).is_none());
    }
    txn.commit().unwrap();
    assert!(shared.read().edges_with_label(&label).is_none());
}

#[test]
fn delete_edge_updates_both_adjacencies() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (a, b, edge) = {
        let mut mutator = txn.mutator();
        let a = empty_node(&mut mutator);
        let b = empty_node(&mut mutator);
        let edge = mutator
            .create_edge(intern("edge.delete").unwrap(), a, b, PropertyMap::new())
            .unwrap();
        mutator.delete_edge(edge).unwrap();
        (a, b, edge)
    };
    txn.commit().unwrap();
    let snapshot = shared.read();
    assert!(!snapshot.is_edge_alive(edge));
    assert!(snapshot.outgoing_edges(a).is_none());
    assert!(snapshot.incoming_edges(b).is_none());
}

#[test]
fn index_entry_dropped_when_bitmap_empties() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let label = intern("node.index.drop-empty").unwrap();
    {
        let mut mutator = txn.mutator();
        let id = mutator
            .create_node(LabelSet::single(label), PropertyMap::new())
            .expect("create_node ok");
        assert_eq!(mutator.read().label_count(), 1);
        mutator.delete_node(id).unwrap();
        assert_eq!(mutator.read().label_count(), 0);
        assert!(!mutator.read().idx_label.contains_key(&label));
    }
    txn.commit().unwrap();
    assert_eq!(shared.read().label_count(), 0);
}

#[test]
fn update_node_with_no_label_changes_does_not_touch_indexes() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let label = intern("node.index.props-only").unwrap();
    let prop = intern("node.index.prop").unwrap();
    {
        let mut mutator = txn.mutator();
        let id = mutator
            .create_node(LabelSet::single(label), PropertyMap::new())
            .expect("create_node ok");
        let before = mutator.read().idx_label.clone();
        mutator
            .update_node(
                id,
                LabelDiff::new([], []).unwrap(),
                PropertyDiff::new([(prop, Value::Int(1))], []).unwrap(),
            )
            .unwrap();
        assert_eq!(mutator.read().idx_label, before);
    }
    txn.commit().unwrap();
}

#[test]
fn read_within_tx_sees_own_writes() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let id = empty_node(&mut mutator);
    assert!(mutator.read().is_node_alive(id));
}

#[test]
fn read_within_tx_sees_label_index_updates() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let label = intern("node.index.tx-read").unwrap();
    let mut mutator = txn.mutator();
    let id = mutator
        .create_node(LabelSet::single(label), PropertyMap::new())
        .expect("create_node ok");
    let row = mutator
        .read()
        .row_for_node_id(id)
        .expect("created node is mapped")
        .get();
    assert!(
        mutator
            .read()
            .nodes_with_label(&label)
            .unwrap()
            .contains(row)
    );
}

#[test]
fn multi_step_tx_emits_changes_in_order() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let id = {
        let mut mutator = txn.mutator();
        let id = empty_node(&mut mutator);
        mutator
            .update_node(
                id,
                LabelDiff::new([intern("node.updated").unwrap()], []).unwrap(),
                PropertyDiff::new([], []).unwrap(),
            )
            .unwrap();
        mutator.delete_node(id).unwrap();
        id
    };
    let outcome = txn.commit().unwrap();
    assert!(matches!(outcome.changes[0], Change::NodeCreated { .. }));
    assert!(matches!(outcome.changes[1], Change::NodeUpdated { .. }));
    assert_eq!(outcome.changes[2], Change::NodeDeleted { id });
}

#[test]
fn extension_event_emits_change_passthrough() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator.extension_event(intern("provider").unwrap(), Arc::from([1_u8, 2]));
    }
    let outcome = txn.commit().unwrap();
    assert!(matches!(
        outcome.changes[0],
        Change::IndexExtensionEvent { .. }
    ));
}

#[test]
fn schema_change_emits_change_passthrough() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator.schema_change(
            GraphId::new(1),
            SchemaChange::GraphDropped {
                id: GraphId::new(2),
            },
        );
    }
    let outcome = txn.commit().unwrap();
    assert!(matches!(outcome.changes[0], Change::SchemaChanged { .. }));
}

#[test]
fn update_edge_updates_properties() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (edge, prop) = {
        let mut mutator = txn.mutator();
        let a = empty_node(&mut mutator);
        let b = empty_node(&mut mutator);
        let edge = mutator
            .create_edge(intern("edge.update").unwrap(), a, b, PropertyMap::new())
            .unwrap();
        let prop = intern("edge.prop").unwrap();
        mutator
            .update_edge(
                edge,
                PropertyDiff::new([(prop, Value::String(prop))], []).unwrap(),
            )
            .unwrap();
        (edge, prop)
    };
    txn.commit().unwrap();
    assert_eq!(
        shared.read().edge_properties(edge).unwrap().get(&prop),
        Some(&Value::String(prop))
    );
}

#[test]
fn remove_node_property_removes_value_and_emits_change() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (id, prop) = {
        let mut mutator = txn.mutator();
        let prop = intern("node.remove.prop").unwrap();
        let id = mutator
            .create_node(
                LabelSet::new(),
                PropertyMap::from_pairs([(prop, Value::Int(7))]).unwrap(),
            )
            .unwrap();
        mutator.remove_node_property(id, prop).unwrap();
        assert!(
            mutator
                .read()
                .node_properties(id)
                .unwrap()
                .get(&prop)
                .is_none()
        );
        (id, prop)
    };
    let outcome = txn.commit().unwrap();
    assert_eq!(
        outcome.changes[1],
        Change::NodePropertyRemoved { id, property: prop }
    );
    assert!(
        shared
            .read()
            .node_properties(id)
            .unwrap()
            .get(&prop)
            .is_none()
    );
}

#[test]
fn remove_node_property_absent_is_noop() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let id = empty_node(&mut mutator);
        mutator
            .remove_node_property(id, intern("node.remove.absent").unwrap())
            .unwrap();
    }
    let outcome = txn.commit().unwrap();
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::NodeCreated { .. }]
    ));
}

#[test]
fn remove_node_label_updates_index_and_emits_change() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (id, label) = {
        let mut mutator = txn.mutator();
        let label = intern("node.remove.label").unwrap();
        let id = mutator
            .create_node(LabelSet::single(label), PropertyMap::new())
            .unwrap();
        mutator.remove_node_label(id, label).unwrap();
        assert!(mutator.read().nodes_with_label(&label).is_none());
        (id, label)
    };
    let outcome = txn.commit().unwrap();
    assert_eq!(outcome.changes[1], Change::NodeLabelRemoved { id, label });
    assert!(shared.read().nodes_with_label(&label).is_none());
}

#[test]
fn remove_edge_property_removes_value_and_emits_change() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (edge, prop) = {
        let mut mutator = txn.mutator();
        let left = empty_node(&mut mutator);
        let right = empty_node(&mut mutator);
        let prop = intern("edge.remove.prop").unwrap();
        let edge = mutator
            .create_edge(
                intern("edge.remove").unwrap(),
                left,
                right,
                PropertyMap::from_pairs([(prop, Value::Int(9))]).unwrap(),
            )
            .unwrap();
        mutator.remove_edge_property(edge, prop).unwrap();
        assert!(
            mutator
                .read()
                .edge_properties(edge)
                .unwrap()
                .get(&prop)
                .is_none()
        );
        (edge, prop)
    };
    let outcome = txn.commit().unwrap();
    assert_eq!(
        outcome.changes.last(),
        Some(&Change::EdgePropertyRemoved {
            id: edge,
            property: prop,
        })
    );
    assert!(
        shared
            .read()
            .edge_properties(edge)
            .unwrap()
            .get(&prop)
            .is_none()
    );
}

#[test]
fn remove_node_property_rejects_immutable_property() {
    let serial = intern("node.remove.immutable.serial").unwrap();
    let person = intern("node.remove.immutable.person").unwrap();
    let graph_type = GraphTypeDef {
        name: intern("node.remove.immutable.graph").unwrap(),
        node_types: vec![NodeTypeDef {
            name: person,
            key_labels: LabelSet::single(person),
            properties: vec![PropertyTypeDef {
                name: serial,
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: false,
                default: None,
                immutable: true,

                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    };
    let shared = SharedGraph::builder(GraphId::new(1))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    let id = txn
        .mutator()
        .create_node(
            LabelSet::single(person),
            PropertyMap::from_pairs([(serial, Value::String(serial))]).unwrap(),
        )
        .unwrap();
    txn.commit().unwrap();

    let mut txn = shared.begin_write();
    let err = txn
        .mutator()
        .remove_node_property(id, serial)
        .expect_err("immutable property removal is rejected");
    assert!(matches!(
        err,
        GraphError::TypeViolation(TypeViolation::ImmutablePropertyUpdate {
            entity_id,
            property,
            ..
        }) if entity_id == EntityId::Node(id) && property == serial
    ));
}

proptest! {
    #[test]
    fn create_delete_sequence_preserves_alive_count(ops in proptest::collection::vec(any::<bool>(), 1..64)) {
        let shared = SharedGraph::new(GraphId::new(1));
        let mut txn = shared.begin_write();
        let mut expected_alive = BTreeSet::new();
        let mut created = Vec::new();
        {
            let mut mutator = txn.mutator();
            for delete_previous in ops {
                let id = mutator
                    .create_node(LabelSet::new(), PropertyMap::new())
                    .expect("create_node ok");
                expected_alive.insert(id);
                created.push(id);
                if delete_previous
                    && let Some(to_delete) = created.first().copied()
                    && expected_alive.remove(&to_delete)
                {
                    mutator.delete_node(to_delete).unwrap();
                }
            }
            prop_assert_eq!(mutator.read().node_count(), expected_alive.len());
            prop_assert_eq!(
                mutator.read().meta.next_node_id,
                1,
                "working meta is updated at commit, allocator advances during mutation"
            );
        }
        let outcome = txn.commit().unwrap();
        prop_assert_eq!(shared.read().node_count(), expected_alive.len());
        prop_assert_eq!(outcome.next_node_id as usize, created.len() + 1);
    }
}

proptest! {
    #[test]
    fn label_index_matches_alive_labeled_nodes(
        ops in proptest::collection::vec((0_u8..3, 0_usize..4, 0_usize..64), 1..64)
    ) {
        let shared = SharedGraph::new(GraphId::new(1));
        let labels = [
            intern("prop.label.a").unwrap(),
            intern("prop.label.b").unwrap(),
            intern("prop.label.c").unwrap(),
            intern("prop.label.d").unwrap(),
        ];
        let mut txn = shared.begin_write();
        let mut alive: std::collections::BTreeMap<NodeId, BTreeSet<IStr>> =
            std::collections::BTreeMap::new();
        let mut created = Vec::new();
        {
            let mut mutator = txn.mutator();
            for (op, label_index, node_index) in ops {
                match op {
                    0 => {
                        let label = labels[label_index % labels.len()];
                        let id = mutator
                            .create_node(LabelSet::single(label), PropertyMap::new())
                            .expect("create_node ok");
                        created.push(id);
                        alive.insert(id, BTreeSet::from([label]));
                    }
                    1 if !alive.is_empty() => {
                        let id = *alive.keys().nth(node_index % alive.len()).unwrap();
                        let label = labels[label_index % labels.len()];
                        let current = alive.get_mut(&id).unwrap();
                        let (added, removed) = if current.contains(&label) {
                            current.remove(&label);
                            (Vec::new(), vec![label])
                        } else {
                            current.insert(label);
                            (vec![label], Vec::new())
                        };
                        mutator
                            .update_node(
                                id,
                                LabelDiff::new(added, removed).unwrap(),
                                PropertyDiff::new([], []).unwrap(),
                            )
                            .unwrap();
                    }
                    2 if !alive.is_empty() => {
                        let id = *alive.keys().nth(node_index % alive.len()).unwrap();
                        mutator.delete_node(id).unwrap();
                        alive.remove(&id);
                    }
                    _ => {}
                }
            }

            let mut expected: std::collections::BTreeMap<IStr, RoaringBitmap> =
                std::collections::BTreeMap::new();
            for (id, node_labels) in &alive {
                // BRIEF-Item-4c: rows are append-assigned, so resolve each id's row
                // through the authoritative map rather than `id - 1` arithmetic.
                let row = mutator.read().row_for_node_id(*id).unwrap().get();
                for label in node_labels {
                    expected.entry(*label).or_default().insert(row);
                }
            }
            for label in labels {
                let expected_bitmap = expected.get(&label);
                let actual_bitmap = mutator.read().nodes_with_label(&label);
                prop_assert_eq!(actual_bitmap, expected_bitmap);
            }
            prop_assert_eq!(
                mutator.read().label_count(),
                expected.values().filter(|bitmap| !bitmap.is_empty()).count()
            );
        }
        let outcome = txn.commit().unwrap();
        prop_assert_eq!(outcome.next_node_id as usize, created.len() + 1);
    }
}

#[test]
#[cfg(not(miri))]
fn four_writer_stress_no_double_allocation() {
    let shared = Arc::new(SharedGraph::new(GraphId::new(1)));
    let nodes_per_thread = 64;
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let shared = Arc::clone(&shared);
            scope.spawn(move || {
                let mut txn = shared.begin_write();
                {
                    let mut mutator = txn.mutator();
                    for _ in 0..nodes_per_thread {
                        mutator
                            .create_node(LabelSet::new(), PropertyMap::new())
                            .expect("create_node ok");
                    }
                }
                txn.commit().unwrap();
            });
        }
    });
    let snapshot = shared.read();
    assert_eq!(snapshot.node_count(), 4 * nodes_per_thread);
    assert_eq!(
        snapshot.meta.next_node_id,
        (4 * nodes_per_thread + 1) as u64
    );
}

#[test]
fn value_type_import_smoke_keeps_schema_deferred() {
    let value_type = ValueType::predefined(PredefinedValueType::String);
    assert_eq!(value_type.predefined, Some(PredefinedValueType::String));
}
