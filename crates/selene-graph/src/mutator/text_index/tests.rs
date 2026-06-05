use selene_core::{
    GraphId, IStr, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap, Value, intern,
};

use crate::{GraphError, SharedGraph};

fn istr(value: &str) -> IStr {
    intern(value).unwrap()
}

fn props(pairs: impl IntoIterator<Item = (IStr, Value)>) -> PropertyMap {
    PropertyMap::from_pairs(pairs).unwrap()
}

fn hit_ids(graph: &SharedGraph, label: &IStr, property: &IStr, query: &str) -> Vec<NodeId> {
    graph
        .read()
        .text_index_for(label, property)
        .unwrap()
        .search(query, 10)
        .into_iter()
        .map(|hit| hit.node_id)
        .collect()
}

#[test]
fn text_index_tracks_create_update_and_delete_documents() {
    let shared = SharedGraph::new(GraphId::new(9101));
    let label = istr("text.index.doc");
    let property = istr("body");
    let other = istr("text.index.other");
    let (doc_a, doc_b) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let doc_a = mutator
            .create_node(
                LabelSet::single(label.clone()),
                props([(property.clone(), Value::String(istr("alpha beta")))]),
            )
            .unwrap();
        let doc_b = mutator
            .create_node(
                LabelSet::single(label.clone()),
                props([(other, Value::Int(9))]),
            )
            .unwrap();
        txn.commit().unwrap();
        (doc_a, doc_b)
    };

    shared
        .create_text_index(label.clone(), property.clone())
        .unwrap();
    assert_eq!(hit_ids(&shared, &label, &property, "alpha"), vec![doc_a]);

    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .update_node(
                doc_b,
                LabelDiff::new([], []).unwrap(),
                PropertyDiff::new([(property.clone(), Value::String(istr("gamma alpha")))], [])
                    .unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    assert_eq!(hit_ids(&shared, &label, &property, "gamma"), vec![doc_b]);

    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .update_node(
                doc_a,
                LabelDiff::new([], []).unwrap(),
                PropertyDiff::new([(property.clone(), Value::String(istr("delta")))], []).unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    assert_eq!(
        hit_ids(&shared, &label, &property, "beta"),
        Vec::<NodeId>::new()
    );

    {
        let mut txn = shared.begin_write();
        txn.mutator().delete_node(doc_b).unwrap();
        txn.commit().unwrap();
    }
    assert_eq!(
        shared
            .read()
            .text_index_for(&label, &property)
            .unwrap()
            .stats()
            .documents,
        1
    );
    assert_eq!(
        hit_ids(&shared, &label, &property, "gamma"),
        Vec::<NodeId>::new()
    );
}

#[test]
fn create_text_index_rejects_duplicate_and_drop_is_idempotent() {
    let shared = SharedGraph::new(GraphId::new(9102));
    let label = istr("text.index.duplicate");
    let property = istr("body");

    shared
        .create_text_index_named(label.clone(), property.clone(), Some(istr("body_idx")))
        .unwrap();
    let err = shared
        .create_text_index(label.clone(), property.clone())
        .unwrap_err();
    assert!(matches!(
        err,
        GraphError::TextIndexAlreadyExists {
            label: err_label,
            property: err_property,
        } if err_label == label && err_property == property
    ));
    assert_eq!(shared.read().text_index_count(), 1);
    assert_eq!(
        shared
            .read()
            .iter_text_index_entries()
            .next()
            .and_then(|(_, _, _, _, name)| name),
        Some(istr("body_idx"))
    );

    shared
        .drop_text_index(label.clone(), property.clone())
        .unwrap();
    shared.drop_text_index(label, property).unwrap();
    assert_eq!(shared.read().text_index_count(), 0);
}

#[test]
fn text_index_ignores_non_string_values_and_tracks_label_membership() {
    let shared = SharedGraph::new(GraphId::new(9103));
    let label = istr("text.index.string.only");
    let other_label = istr("text.index.other.label");
    let property = istr("body");
    let doc = {
        let mut txn = shared.begin_write();
        let doc = txn
            .mutator()
            .create_node(
                LabelSet::single(other_label.clone()),
                props([(property.clone(), Value::Int(42))]),
            )
            .unwrap();
        txn.commit().unwrap();
        doc
    };

    shared
        .create_text_index(label.clone(), property.clone())
        .unwrap();
    assert_eq!(
        shared
            .read()
            .text_index_for(&label, &property)
            .unwrap()
            .stats()
            .documents,
        0
    );

    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .update_node(
                doc,
                LabelDiff::new([label.clone()], [other_label]).unwrap(),
                PropertyDiff::new([(property.clone(), Value::String(istr("needle hay")))], [])
                    .unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    assert_eq!(hit_ids(&shared, &label, &property, "needle"), vec![doc]);

    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .update_node(
                doc,
                LabelDiff::new([], []).unwrap(),
                PropertyDiff::new([(property.clone(), Value::Int(7))], []).unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    assert_eq!(
        shared
            .read()
            .text_index_for(&label, &property)
            .unwrap()
            .stats()
            .documents,
        0
    );
}
