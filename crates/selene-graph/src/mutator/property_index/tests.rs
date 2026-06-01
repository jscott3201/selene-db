use selene_core::{
    Change, GraphId, IStr, LabelSet, PropertyMap, SchemaChange, SchemaPropertyIndexKind, Value,
    intern,
};

use crate::{GraphError, SharedGraph, TypedIndexKind};

fn istr(value: &str) -> IStr {
    intern(value).unwrap()
}

fn props(pairs: impl IntoIterator<Item = (IStr, Value)>) -> PropertyMap {
    PropertyMap::from_pairs(pairs).unwrap()
}

#[test]
fn create_property_index_updates_working_graph_and_emits_schema_change() {
    let shared = SharedGraph::new(GraphId::new(4201));
    let label = istr("mutator.index.person");
    let property = istr("mutator.index.age");
    let outcome = {
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node(
                    LabelSet::single(label.clone()),
                    props([(property.clone(), Value::Int(42))]),
                )
                .unwrap();
            mutator
                .create_property_index(label.clone(), property.clone(), TypedIndexKind::I64)
                .unwrap();
            let rows = mutator
                .read()
                .nodes_with_property_eq(&label, &property, &Value::Int(42))
                .unwrap();
            assert_eq!(rows.iter().collect::<Vec<_>>(), vec![0]);
        }
        txn.commit().unwrap()
    };

    assert!(
        shared
            .read()
            .property_index_for(&label, &property)
            .is_some()
    );
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::NodeCreated { .. }, Change::SchemaChanged {
            graph,
            change: SchemaChange::PropertyIndexCreatedNamed {
                label: changed_label,
                property: changed_property,
                kind: SchemaPropertyIndexKind::I64,
                name: None,
            },
        }] if *graph == GraphId::new(4201)
            && *changed_label == label
            && *changed_property == property
    ));
}

#[test]
fn create_property_index_rejects_duplicate_in_working_graph() {
    let shared = SharedGraph::new(GraphId::new(4202));
    let label = istr("mutator.index.duplicate");
    let property = istr("mutator.index.prop");
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    mutator
        .create_property_index(label.clone(), property.clone(), TypedIndexKind::I64)
        .unwrap();

    let err = mutator
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
fn create_property_index_rejects_kind_mismatch_on_existing_node() {
    let shared = SharedGraph::new(GraphId::new(4203));
    let label = istr("mutator.index.kind");
    let property = istr("mutator.index.value");
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    mutator
        .create_node(
            LabelSet::single(label.clone()),
            props([(property.clone(), Value::String(istr("not-an-int")))]),
        )
        .unwrap();

    let err = mutator
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
    assert!(
        mutator
            .read()
            .property_index_for(&label, &property)
            .is_none()
    );
}

#[test]
fn drop_property_index_removes_from_working_graph_and_emits_schema_change() {
    let shared = SharedGraph::new(GraphId::new(4204));
    let label = istr("mutator.index.drop");
    let property = istr("mutator.index.prop");
    shared
        .create_property_index(label.clone(), property.clone(), TypedIndexKind::I64)
        .unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .drop_property_index(label.clone(), property.clone())
            .unwrap();
        txn.commit().unwrap()
    };

    assert!(
        shared
            .read()
            .property_index_for(&label, &property)
            .is_none()
    );
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::SchemaChanged {
            graph,
            change: SchemaChange::PropertyIndexDropped {
                label: changed_label,
                property: changed_property,
            },
        }] if *graph == GraphId::new(4204)
            && *changed_label == label
            && *changed_property == property
    ));
}

#[test]
fn drop_property_index_is_idempotent_and_emits_no_change_when_absent() {
    let shared = SharedGraph::new(GraphId::new(4205));
    let label = istr("mutator.index.absent");
    let property = istr("mutator.index.prop");
    let outcome = {
        let mut txn = shared.begin_write();
        txn.mutator().drop_property_index(label, property).unwrap();
        txn.commit().unwrap()
    };

    assert!(outcome.changes.is_empty());
    assert_eq!(shared.read().property_index_count(), 0);
}

#[test]
fn create_then_drop_in_same_transaction_leaves_no_index() {
    let shared = SharedGraph::new(GraphId::new(4206));
    let label = istr("mutator.index.roundtrip");
    let property = istr("mutator.index.prop");
    let outcome = {
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_property_index(label.clone(), property.clone(), TypedIndexKind::I64)
                .unwrap();
            mutator
                .drop_property_index(label.clone(), property.clone())
                .unwrap();
            assert!(
                mutator
                    .read()
                    .property_index_for(&label, &property)
                    .is_none()
            );
        }
        txn.commit().unwrap()
    };

    assert_eq!(outcome.changes.len(), 2);
    assert!(
        shared
            .read()
            .property_index_for(&label, &property)
            .is_none()
    );
}

#[test]
fn rollback_discards_created_property_index() {
    let shared = SharedGraph::new(GraphId::new(4207));
    let label = istr("mutator.index.rollback");
    let property = istr("mutator.index.prop");
    let mut txn = shared.begin_write();
    txn.mutator()
        .create_property_index(label.clone(), property.clone(), TypedIndexKind::I64)
        .unwrap();
    txn.rollback();

    assert!(
        shared
            .read()
            .property_index_for(&label, &property)
            .is_none()
    );
}
