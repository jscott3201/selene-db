use proptest::prelude::*;
use roaring::RoaringBitmap;
use selene_core::{
    GraphId, IStr, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap, Value, intern,
};

use crate::{GraphError, SeleneGraph, SharedGraph, TypedIndexKind};

#[test]
fn create_property_index_builds_from_existing_nodes() {
    let shared = SharedGraph::new(GraphId::new(1));
    let person = intern("prop.index.person").unwrap();
    let order = intern("prop.index.order").unwrap();
    let age = intern("prop.index.age").unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(person.clone()),
                property_map([(age.clone(), Value::Int(30))]),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(person.clone()),
                property_map([(age.clone(), Value::Int(40))]),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(order),
                property_map([(age.clone(), Value::Int(30))]),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    shared
        .create_property_index(person.clone(), age.clone(), TypedIndexKind::I64)
        .unwrap();

    let snapshot = shared.read();
    let rows = snapshot
        .nodes_with_property_eq(&person, &age, &Value::Int(30))
        .unwrap();
    assert_eq!(rows.iter().collect::<Vec<_>>(), vec![0]);
    assert_eq!(snapshot.property_index_count(), 1);
}

#[test]
fn create_property_index_rejects_duplicates() {
    let shared = SharedGraph::new(GraphId::new(1));
    let label = intern("prop.duplicate.label").unwrap();
    let property = intern("prop.duplicate.property").unwrap();
    shared
        .create_property_index(label.clone(), property.clone(), TypedIndexKind::I64)
        .unwrap();

    let err = shared
        .create_property_index(label.clone(), property.clone(), TypedIndexKind::I64)
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::PropertyIndexAlreadyExists {
            label: err_label,
            property: err_property,
        } if err_label == label && err_property == property
    ));
}

#[test]
fn create_property_index_rejects_existing_kind_violations() {
    let shared = SharedGraph::new(GraphId::new(1));
    let label = intern("prop.strict.label").unwrap();
    let property = intern("prop.strict.property").unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(label.clone()),
                property_map([(property.clone(), Value::String(intern("wrong").unwrap()))]),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let err = shared
        .create_property_index(label.clone(), property.clone(), TypedIndexKind::I64)
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::IndexValueRejected {
            label: err_label,
            property: err_property,
            expected_kind: TypedIndexKind::I64,
            observed: "String",
        } if err_label == label && err_property == property
    ));
}

#[test]
fn drop_property_index_is_idempotent() {
    let shared = SharedGraph::new(GraphId::new(1));
    let label = intern("prop.drop.label").unwrap();
    let property = intern("prop.drop.property").unwrap();
    shared
        .create_property_index(label.clone(), property.clone(), TypedIndexKind::I64)
        .unwrap();
    assert_eq!(shared.read().property_index_count(), 1);

    shared
        .drop_property_index(label.clone(), property.clone())
        .unwrap();
    shared.drop_property_index(label, property).unwrap();

    assert_eq!(shared.read().property_index_count(), 0);
}

#[test]
fn property_index_tracks_create_update_delete_in_one_commit() {
    let shared = SharedGraph::new(GraphId::new(1));
    let label = intern("prop.track.label").unwrap();
    let property = intern("prop.track.property").unwrap();
    shared
        .create_property_index(label.clone(), property.clone(), TypedIndexKind::I64)
        .unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let first = mutator
            .create_node(
                LabelSet::single(label.clone()),
                property_map([(property.clone(), Value::Int(30))]),
            )
            .unwrap();
        let second = mutator
            .create_node(
                LabelSet::single(label.clone()),
                property_map([(property.clone(), Value::Int(30))]),
            )
            .unwrap();
        mutator
            .update_node(
                first,
                LabelDiff::new([], []).unwrap(),
                PropertyDiff::new([(property.clone(), Value::Int(31))], []).unwrap(),
            )
            .unwrap();
        mutator.delete_node(second).unwrap();
    }
    txn.commit().unwrap();

    let snapshot = shared.read();
    assert_eq!(
        snapshot
            .nodes_with_property_eq(&label, &property, &Value::Int(31))
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        vec![0]
    );
    assert!(
        snapshot
            .nodes_with_property_eq(&label, &property, &Value::Int(30))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn property_range_lookup_unions_matching_keys() {
    let shared = SharedGraph::new(GraphId::new(1));
    let label = intern("prop.range.label").unwrap();
    let property = intern("prop.range.property").unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        for value in [1, 2, 3, 4] {
            mutator
                .create_node(
                    LabelSet::single(label.clone()),
                    property_map([(property.clone(), Value::Int(value))]),
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }
    shared
        .create_property_index(label.clone(), property.clone(), TypedIndexKind::I64)
        .unwrap();

    let rows = shared
        .read()
        .nodes_with_property_range(&label, &property, Value::Int(2)..Value::Int(4))
        .unwrap();

    assert_eq!(rows.iter().collect::<Vec<_>>(), vec![1, 2]);
}

#[test]
fn property_prefix_lookup_is_string_only() {
    let shared = SharedGraph::new(GraphId::new(1));
    let label = intern("prop.prefix.label").unwrap();
    let name = intern("prop.prefix.name").unwrap();
    let age = intern("prop.prefix.age").unwrap();
    let ada = intern("Ada Lovelace").unwrap();
    let grace = intern("Grace Hopper").unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(label.clone()),
                property_map([
                    (name.clone(), Value::String(ada)),
                    (age.clone(), Value::Int(36)),
                ]),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(label.clone()),
                property_map([
                    (name.clone(), Value::String(grace)),
                    (age.clone(), Value::Int(85)),
                ]),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    shared
        .create_property_index(label.clone(), name.clone(), TypedIndexKind::String)
        .unwrap();
    shared
        .create_property_index(label.clone(), age.clone(), TypedIndexKind::I64)
        .unwrap();

    let snapshot = shared.read();
    let rows = snapshot
        .nodes_with_property_prefix(&label, &name, "Ada")
        .unwrap();
    assert_eq!(rows.iter().collect::<Vec<_>>(), vec![0]);
    assert!(
        snapshot
            .nodes_with_property_prefix(&label, &age, "8")
            .is_none()
    );
}

#[test]
fn cross_label_property_indexes_are_independent() {
    let shared = SharedGraph::new(GraphId::new(1));
    let person = intern("prop.cross.person").unwrap();
    let order = intern("prop.cross.order").unwrap();
    let age = intern("prop.cross.age").unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(person.clone()),
                property_map([(age.clone(), Value::Int(30))]),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(order.clone()),
                property_map([(age.clone(), Value::Int(30))]),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    shared
        .create_property_index(person.clone(), age.clone(), TypedIndexKind::I64)
        .unwrap();
    shared
        .create_property_index(order.clone(), age.clone(), TypedIndexKind::I64)
        .unwrap();

    let snapshot = shared.read();
    assert_eq!(
        snapshot
            .nodes_with_property_eq(&person, &age, &Value::Int(30))
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        vec![0]
    );
    assert_eq!(
        snapshot
            .nodes_with_property_eq(&order, &age, &Value::Int(30))
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        vec![1]
    );
}

#[test]
fn old_snapshot_does_not_see_new_property_index() {
    let shared = SharedGraph::new(GraphId::new(1));
    let label = intern("prop.snapshot.label").unwrap();
    let property = intern("prop.snapshot.property").unwrap();
    let old = shared.read();

    shared
        .create_property_index(label, property, TypedIndexKind::I64)
        .unwrap();

    assert_eq!(old.property_index_count(), 0);
    assert_eq!(shared.read().property_index_count(), 1);
}

proptest! {
    #[test]
    fn indexed_i64_sequence_matches_column_scan(ops in prop::collection::vec(0u8..32, 1..40)) {
        let shared = SharedGraph::new(GraphId::new(1));
        let label = intern("prop.sequence.label").unwrap();
        let property = intern("prop.sequence.property").unwrap();
        shared.create_property_index(label.clone(), property.clone(), TypedIndexKind::I64).unwrap();
        let mut nodes: Vec<(NodeId, bool)> = Vec::new();

        for op in ops {
            let live_positions = nodes
                .iter()
                .enumerate()
                .filter_map(|(idx, (_, alive))| (*alive).then_some(idx))
                .collect::<Vec<_>>();
            let mut txn = shared.begin_write();
            {
                let mut mutator = txn.mutator();
                match op % 4 {
                    0 | 3 => {
                        let id = mutator.create_node(
                            LabelSet::single(label.clone()),
                            property_map([(property.clone(), Value::Int((op % 5) as i64))]),
                        ).unwrap();
                        nodes.push((id, true));
                    }
                    1 if !live_positions.is_empty() => {
                        let pos = live_positions[(op as usize) % live_positions.len()];
                        mutator.update_node(
                            nodes[pos].0,
                            LabelDiff::new([], []).unwrap(),
                            PropertyDiff::new([(property.clone(), Value::Int(((op + 1) % 5) as i64))], []).unwrap(),
                        ).unwrap();
                    }
                    2 if !live_positions.is_empty() => {
                        let pos = live_positions[(op as usize) % live_positions.len()];
                        mutator.delete_node(nodes[pos].0).unwrap();
                        nodes[pos].1 = false;
                    }
                    _ => {}
                }
            }
            txn.commit().unwrap();
            let snapshot = shared.read();
            for value in 0..5 {
                let expected = brute_force_i64(&snapshot, label.clone(), property.clone(), value);
                let actual = snapshot
                    .nodes_with_property_eq(&label, &property, &Value::Int(value))
                    .unwrap();
                prop_assert_eq!(
                    actual.iter().collect::<Vec<_>>(),
                    expected.iter().collect::<Vec<_>>()
                );
            }
        }
    }
}

fn property_map(pairs: impl IntoIterator<Item = (IStr, Value)>) -> PropertyMap {
    PropertyMap::from_pairs(pairs).unwrap()
}

fn brute_force_i64(graph: &SeleneGraph, label: IStr, property: IStr, value: i64) -> RoaringBitmap {
    let mut rows = RoaringBitmap::new();
    for row in graph.node_store.alive.iter() {
        let labels = graph
            .node_store
            .labels
            .get(row as usize)
            .expect("alive row has labels");
        let props = graph
            .node_store
            .properties
            .get(row as usize)
            .expect("alive row has properties");
        if labels.contains(&label)
            && matches!(props.get(&property), Some(Value::Int(found)) if *found == value)
        {
            rows.insert(row);
        }
    }
    rows
}
