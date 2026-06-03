use selene_core::{
    Change, GraphId, IStr, LabelSet, PropertyMap, SchemaChange, SchemaVectorIndexKind, Value,
    VectorValue, intern,
};

use crate::{GraphError, SharedGraph, VectorIndexKind};

fn istr(value: &str) -> IStr {
    intern(value).unwrap()
}

fn vector(components: &[f32]) -> Value {
    Value::Vector(VectorValue::new(components.to_vec()).unwrap())
}

fn props(pairs: impl IntoIterator<Item = (IStr, Value)>) -> PropertyMap {
    PropertyMap::from_pairs(pairs).unwrap()
}

#[test]
fn create_vector_index_updates_working_graph_and_emits_schema_change() {
    let shared = SharedGraph::new(GraphId::new(8201));
    let label = istr("mutator.vector.doc");
    let property = istr("embedding");
    let name = istr("doc_embedding_idx");
    let outcome = {
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node(
                    LabelSet::single(label.clone()),
                    props([(property.clone(), vector(&[1.0, 2.0, 3.0]))]),
                )
                .unwrap();
            mutator
                .create_vector_index_named(
                    label.clone(),
                    property.clone(),
                    VectorIndexKind::Flat,
                    3,
                    Some(name.clone()),
                )
                .unwrap();
            assert_eq!(
                mutator
                    .read()
                    .vector_index_for(&label, &property)
                    .unwrap()
                    .cardinality(),
                1
            );
        }
        txn.commit().unwrap()
    };

    assert!(shared.read().vector_index_for(&label, &property).is_some());
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::NodeCreated { .. }, Change::SchemaChanged {
            graph,
            change: SchemaChange::VectorIndexCreated {
                label: changed_label,
                property: changed_property,
                kind: SchemaVectorIndexKind::Flat,
                dimension: 3,
                name: Some(changed_name),
            },
        }] if *graph == GraphId::new(8201)
            && *changed_label == label
            && *changed_property == property
            && *changed_name == name
    ));
}

#[test]
fn create_vector_index_rejects_duplicate_in_working_graph() {
    let shared = SharedGraph::new(GraphId::new(8202));
    let label = istr("mutator.vector.duplicate");
    let property = istr("embedding");
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    mutator
        .create_vector_index(label.clone(), property.clone(), VectorIndexKind::Flat, 3)
        .unwrap();

    let err = mutator
        .create_vector_index(label.clone(), property.clone(), VectorIndexKind::Flat, 3)
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::VectorIndexAlreadyExists {
            label: err_label,
            property: err_property,
        } if err_label == label && err_property == property
    ));
}

#[test]
fn create_vector_index_rejects_zero_dimension() {
    let shared = SharedGraph::new(GraphId::new(8203));
    let label = istr("mutator.vector.zero");
    let property = istr("embedding");
    let err = shared
        .create_vector_index(label, property, VectorIndexKind::Flat, 0)
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::VectorIndexInvalidDimension { dimension: 0 }
    ));
}

#[test]
fn drop_vector_index_removes_from_working_graph_and_emits_schema_change() {
    let shared = SharedGraph::new(GraphId::new(8204));
    let label = istr("mutator.vector.drop");
    let property = istr("embedding");
    shared
        .create_vector_index(label.clone(), property.clone(), VectorIndexKind::Flat, 3)
        .unwrap();
    let outcome = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .drop_vector_index(label.clone(), property.clone())
            .unwrap();
        txn.commit().unwrap()
    };

    assert!(shared.read().vector_index_for(&label, &property).is_none());
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::SchemaChanged {
            graph,
            change: SchemaChange::VectorIndexDropped {
                label: changed_label,
                property: changed_property,
            },
        }] if *graph == GraphId::new(8204)
            && *changed_label == label
            && *changed_property == property
    ));
}

#[test]
fn drop_vector_index_is_idempotent_and_emits_no_change_when_absent() {
    let shared = SharedGraph::new(GraphId::new(8205));
    let label = istr("mutator.vector.absent");
    let property = istr("embedding");
    let outcome = {
        let mut txn = shared.begin_write();
        txn.mutator().drop_vector_index(label, property).unwrap();
        txn.commit().unwrap()
    };

    assert!(outcome.changes.is_empty());
    assert_eq!(shared.read().vector_index_count(), 0);
}
