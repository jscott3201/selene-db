use selene_core::{
    DbString, GraphId, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap, Value,
};

use crate::{GraphError, SharedGraph};

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).unwrap()
}

fn props(pairs: impl IntoIterator<Item = (DbString, Value)>) -> PropertyMap {
    PropertyMap::from_pairs(pairs).unwrap()
}

fn hit_ids(graph: &SharedGraph, label: &DbString, property: &DbString, query: &str) -> Vec<NodeId> {
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
    let label = db_string("text.index.doc");
    let property = db_string("body");
    let other = db_string("text.index.other");
    let (doc_a, doc_b) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let doc_a = mutator
            .create_node(
                LabelSet::single(label.clone()),
                props([(property.clone(), Value::String(db_string("alpha beta")))]),
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
                PropertyDiff::new(
                    [(property.clone(), Value::String(db_string("gamma alpha")))],
                    [],
                )
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
                PropertyDiff::new([(property.clone(), Value::String(db_string("delta")))], [])
                    .unwrap(),
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
fn text_index_update_preserves_unrelated_registered_property() {
    let shared = SharedGraph::new(GraphId::new(9104));
    let label = db_string("text.index.multi.property");
    let body = db_string("body");
    let summary = db_string("summary");
    let doc = {
        let mut txn = shared.begin_write();
        let doc = txn
            .mutator()
            .create_node(
                LabelSet::single(label.clone()),
                props([
                    (body.clone(), Value::String(db_string("alpha body"))),
                    (summary.clone(), Value::String(db_string("stable summary"))),
                ]),
            )
            .unwrap();
        txn.commit().unwrap();
        doc
    };

    shared
        .create_text_index(label.clone(), body.clone())
        .unwrap();
    shared
        .create_text_index(label.clone(), summary.clone())
        .unwrap();
    assert_eq!(hit_ids(&shared, &label, &body, "alpha"), vec![doc]);
    assert_eq!(hit_ids(&shared, &label, &summary, "stable"), vec![doc]);

    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .update_node(
                doc,
                LabelDiff::new([], []).unwrap(),
                PropertyDiff::new([(body.clone(), Value::String(db_string("beta body")))], [])
                    .unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    assert_eq!(hit_ids(&shared, &label, &body, "beta"), vec![doc]);
    assert_eq!(
        hit_ids(&shared, &label, &body, "alpha"),
        Vec::<NodeId>::new()
    );
    assert_eq!(hit_ids(&shared, &label, &summary, "stable"), vec![doc]);
}

#[test]
fn create_text_index_rejects_duplicate_and_drop_is_idempotent() {
    let shared = SharedGraph::new(GraphId::new(9102));
    let label = db_string("text.index.duplicate");
    let property = db_string("body");

    shared
        .create_text_index_named(label.clone(), property.clone(), Some(db_string("body_idx")))
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
        Some(db_string("body_idx"))
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
    let label = db_string("text.index.string.only");
    let other_label = db_string("text.index.other.label");
    let property = db_string("body");
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
                PropertyDiff::new(
                    [(property.clone(), Value::String(db_string("needle hay")))],
                    [],
                )
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
