use selene_core::{
    CancellationChecker, GraphId, IStr, LabelDiff, LabelSet, PropertyDiff, PropertyMap, Value,
    VectorMetric, VectorValue, intern,
};

use super::VectorIndex;
use crate::{ApproximateVectorSearchOptions, GraphError, SharedGraph, VectorIndexKind};

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
fn vector_index_tracks_create_update_and_delete_membership() {
    let shared = SharedGraph::new(GraphId::new(8101));
    let label = istr("vector.index.doc");
    let property = istr("embedding");
    let other = istr("vector.index.other");
    let (doc_a, doc_b) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let doc_a = mutator
            .create_node(
                LabelSet::single(label.clone()),
                props([(property.clone(), vector(&[1.0, 0.0]))]),
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
        .create_vector_index(label.clone(), property.clone(), VectorIndexKind::Flat, 2)
        .unwrap();
    assert_eq!(
        shared
            .read()
            .vector_index_for(&label, &property)
            .unwrap()
            .rows()
            .iter()
            .collect::<Vec<_>>(),
        vec![0]
    );

    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .update_node(
                doc_b,
                LabelDiff::new([], []).unwrap(),
                PropertyDiff::new([(property.clone(), vector(&[0.0, 1.0]))], []).unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    assert_eq!(
        shared
            .read()
            .vector_index_for(&label, &property)
            .unwrap()
            .rows()
            .iter()
            .collect::<Vec<_>>(),
        vec![0, 1]
    );

    {
        let mut txn = shared.begin_write();
        txn.mutator().delete_node(doc_a).unwrap();
        txn.commit().unwrap();
    }
    assert_eq!(
        shared
            .read()
            .vector_index_for(&label, &property)
            .unwrap()
            .rows()
            .iter()
            .collect::<Vec<_>>(),
        vec![1]
    );
}

#[test]
fn create_vector_index_rejects_existing_wrong_kind() {
    let shared = SharedGraph::new(GraphId::new(8102));
    let label = istr("vector.index.kind");
    let property = istr("embedding");
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(label.clone()),
                props([(property.clone(), Value::String(istr("not-vector")))]),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let err = shared
        .create_vector_index(label.clone(), property.clone(), VectorIndexKind::Flat, 3)
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::VectorIndexValueRejected {
            label: err_label,
            property: err_property,
            expected_dimension: 3,
            observed,
        } if err_label == label && err_property == property && observed == "String"
    ));
}

#[test]
fn create_vector_index_rejects_existing_dimension_mismatch() {
    let shared = SharedGraph::new(GraphId::new(8103));
    let label = istr("vector.index.dimension");
    let property = istr("embedding");
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(label.clone()),
                props([(property.clone(), vector(&[1.0, 2.0]))]),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let err = shared
        .create_vector_index(label.clone(), property.clone(), VectorIndexKind::Flat, 3)
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::VectorIndexValueRejected {
            label: err_label,
            property: err_property,
            expected_dimension: 3,
            observed,
        } if err_label == label && err_property == property && observed == "VECTOR<2>"
    ));
}

#[test]
fn create_hnsw_cosine_index_rejects_existing_zero_norm_vector() {
    let shared = SharedGraph::new(GraphId::new(8106));
    let label = istr("vector.index.cosine.zero");
    let property = istr("embedding");
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(label.clone()),
                props([(property.clone(), vector(&[0.0, 0.0]))]),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let err = shared
        .create_vector_index(
            label.clone(),
            property.clone(),
            VectorIndexKind::HnswCosine,
            2,
        )
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::VectorIndexValueRejected {
            label: err_label,
            property: err_property,
            expected_dimension: 2,
            observed,
        } if err_label == label
            && err_property == property
            && observed.contains("zero-norm vector")
    ));
}

#[test]
fn indexed_vector_property_rejects_later_dimension_drift() {
    let shared = SharedGraph::new(GraphId::new(8104));
    let label = istr("vector.index.strict");
    let property = istr("embedding");
    let doc = {
        let mut txn = shared.begin_write();
        let doc = txn
            .mutator()
            .create_node(LabelSet::single(label.clone()), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        doc
    };
    shared
        .create_vector_index(label.clone(), property.clone(), VectorIndexKind::Flat, 2)
        .unwrap();

    let err = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .update_node(
                doc,
                LabelDiff::new([], []).unwrap(),
                PropertyDiff::new([(property.clone(), vector(&[1.0, 2.0, 3.0]))], []).unwrap(),
            )
            .unwrap_err()
    };

    assert!(matches!(
        err,
        GraphError::VectorIndexValueRejected {
            label: err_label,
            property: err_property,
            expected_dimension: 2,
            observed,
        } if err_label == label && err_property == property && observed == "VECTOR<3>"
    ));
}

#[test]
fn hnsw_vector_index_tracks_membership_and_metric() {
    let shared = SharedGraph::new(GraphId::new(8105));
    let label = istr("vector.index.hnsw");
    let property = istr("embedding");
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(label.clone()),
                props([(property.clone(), vector(&[1.0, 0.0]))]),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(label.clone()),
                props([(property.clone(), vector(&[2.0, 0.0]))]),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    shared
        .create_vector_index(
            label.clone(),
            property.clone(),
            VectorIndexKind::HnswSquaredEuclidean,
            2,
        )
        .unwrap();

    let index = shared.read().vector_index_for(&label, &property).unwrap();
    assert!(index.is_hnsw());
    assert_eq!(index.hnsw_metric(), Some(VectorMetric::SquaredEuclidean));
    assert_eq!(index.rows().iter().collect::<Vec<_>>(), vec![0, 1]);
}

#[test]
fn flat_vector_index_memory_usage_reports_row_bitmap_only() {
    let mut index = VectorIndex::new(VectorIndexKind::Flat, 2).unwrap();
    let first = VectorValue::new(vec![1.0, 0.0]).unwrap();
    let second = VectorValue::new(vec![2.0, 0.0]).unwrap();

    index.insert_value(0, &first).unwrap();
    index.insert_value(70_000, &second).unwrap();

    let usage = index.memory_usage();
    assert_eq!(usage.indexed_rows, 2);
    assert!(usage.row_bitmap_serialized_bytes > 0);
    assert_eq!(usage.hnsw_entries, 0);
    assert_eq!(usage.hnsw_index_bytes, 0);
    assert_eq!(usage.hnsw_referenced_vector_bytes, 0);
    assert_eq!(usage.estimated_reachable_bytes, usage.estimated_index_bytes);
    assert!(usage.estimated_index_bytes >= usage.row_bitmap_bytes);
}

#[test]
fn hnsw_vector_index_memory_usage_reports_links_and_stale_entries() {
    let mut index = VectorIndex::new(VectorIndexKind::HnswSquaredEuclidean, 2).unwrap();
    let vectors: Vec<_> = (0..32)
        .map(|row| VectorValue::new(vec![row as f32, 0.0]).unwrap())
        .collect();
    for (row, vector) in vectors.iter().enumerate() {
        index.insert_value(row as u32, vector).unwrap();
    }

    index.remove_row(7);

    let usage = index.memory_usage();
    assert_eq!(usage.indexed_rows, 31);
    assert_eq!(usage.hnsw_entries, 32);
    assert_eq!(usage.hnsw_live_entries, 31);
    assert_eq!(usage.hnsw_deleted_entries, 1);
    assert!(usage.hnsw_link_count > 0);
    assert!(usage.hnsw_index_bytes > 0);
    assert!(usage.hnsw_referenced_vector_bytes >= 32 * 2 * std::mem::size_of::<f32>());
    assert_eq!(
        usage.estimated_reachable_bytes,
        usage
            .estimated_index_bytes
            .saturating_add(usage.hnsw_referenced_vector_bytes)
    );
}

#[test]
fn shared_rebuild_vector_indexes_reclaims_stale_hnsw_entries() {
    let shared = SharedGraph::new(GraphId::new(8107));
    let label = istr("vector.index.rebuild.doc");
    let property = istr("embedding");
    let ids = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let mut ids = Vec::new();
        for row in 0..48 {
            ids.push(
                mutator
                    .create_node(
                        LabelSet::single(label.clone()),
                        props([(property.clone(), vector(&[row as f32, 0.0]))]),
                    )
                    .unwrap(),
            );
        }
        txn.commit().unwrap();
        ids
    };
    shared
        .create_vector_index(
            label.clone(),
            property.clone(),
            VectorIndexKind::HnswSquaredEuclidean,
            2,
        )
        .unwrap();

    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        for (offset, id) in ids.iter().copied().take(8).enumerate() {
            mutator
                .update_node(
                    id,
                    LabelDiff::new([], []).unwrap(),
                    PropertyDiff::new(
                        [(property.clone(), vector(&[1_000.0 + offset as f32, 0.0]))],
                        [],
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        for id in ids.iter().copied().skip(8).take(4) {
            mutator.delete_node(id).unwrap();
        }
        txn.commit().unwrap();
    }

    let before = shared
        .read()
        .vector_index_for(&label, &property)
        .unwrap()
        .memory_usage();
    assert_eq!(before.indexed_rows, 44);
    assert_eq!(before.hnsw_entries, 56);
    assert_eq!(before.hnsw_live_entries, 44);
    assert_eq!(before.hnsw_deleted_entries, 12);

    let report = shared.rebuild_vector_indexes().unwrap();

    assert_eq!(report.indexes_rebuilt, 1);
    assert_eq!(report.reclaimed_hnsw_entries, 12);
    assert_eq!(report.reclaimed_hnsw_deleted_entries, 12);
    assert!(report.reclaimed_reachable_bytes > 0);
    let entry = &report.entries[0];
    assert_eq!(entry.label, label);
    assert_eq!(entry.property, property);
    assert_eq!(entry.kind, VectorIndexKind::HnswSquaredEuclidean);
    assert_eq!(entry.dimension, 2);
    assert_eq!(entry.before, before);
    assert_eq!(entry.after.indexed_rows, 44);
    assert_eq!(entry.after.hnsw_entries, 44);
    assert_eq!(entry.after.hnsw_live_entries, 44);
    assert_eq!(entry.after.hnsw_deleted_entries, 0);

    let after = shared
        .read()
        .vector_index_for(&label, &property)
        .unwrap()
        .memory_usage();
    assert_eq!(after, entry.after);

    let hits = shared
        .approximate_vector_search_nodes_checked(
            &label,
            &property,
            &VectorValue::new(vec![1_000.0, 0.0]).unwrap(),
            ApproximateVectorSearchOptions::new(VectorMetric::SquaredEuclidean, 1, 64),
            CancellationChecker::disabled(),
        )
        .unwrap();
    assert_eq!(hits[0].node_id, ids[0]);
    assert_eq!(hits[0].distance, 0.0);
}
