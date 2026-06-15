use super::*;

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
    assert_eq!(usage.hnsw_link_count, 0);
    assert_eq!(usage.hnsw_level_zero_link_count, 0);
    assert_eq!(usage.hnsw_upper_layer_link_count, 0);
    assert_eq!(usage.hnsw_max_layer_count, 0);
    assert_eq!(usage.hnsw_max_links_per_layer, 0);
    assert_eq!(usage.hnsw_average_links_per_entry_basis_points, 0);
    assert_eq!(usage.ivf_entries, 0);
    assert_eq!(usage.ivf_index_bytes, 0);
    assert_eq!(usage.ivf_referenced_vector_bytes, 0);
    assert_eq!(usage.ivf_centroids, 0);
    assert_eq!(usage.ivf_list_count, 0);
    assert_eq!(usage.ivf_non_empty_list_count, 0);
    assert_eq!(usage.ivf_max_list_len, 0);
    assert_eq!(usage.ivf_average_list_len_basis_points, 0);
    assert_eq!(usage.ivf_assigned_entries, 0);
    assert_eq!(usage.ivf_pending_retrain_entries, 0);
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
    assert!(usage.hnsw_level_zero_link_count > 0);
    assert_eq!(
        usage
            .hnsw_level_zero_link_count
            .saturating_add(usage.hnsw_upper_layer_link_count),
        usage.hnsw_link_count
    );
    assert!(usage.hnsw_max_layer_count > 0);
    assert!(usage.hnsw_max_links_per_layer > 0);
    assert!(usage.hnsw_average_links_per_entry_basis_points > 0);
    assert!(usage.hnsw_index_bytes > 0);
    assert!(usage.hnsw_referenced_vector_bytes >= 32 * 2 * std::mem::size_of::<f32>());
    assert_eq!(usage.ivf_entries, 0);
    assert_eq!(usage.ivf_index_bytes, 0);
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
    let label = db_string("vector.index.rebuild.doc");
    let property = db_string("embedding");
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
    assert_eq!(
        before
            .hnsw_level_zero_link_count
            .saturating_add(before.hnsw_upper_layer_link_count),
        before.hnsw_link_count
    );
    assert_eq!(
        entry
            .after
            .hnsw_level_zero_link_count
            .saturating_add(entry.after.hnsw_upper_layer_link_count),
        entry.after.hnsw_link_count
    );
    assert!(entry.after.hnsw_level_zero_link_count > 0);
    assert!(entry.after.hnsw_max_layer_count > 0);
    assert!(entry.after.hnsw_max_links_per_layer > 0);
    assert!(entry.after.hnsw_average_links_per_entry_basis_points > 0);

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

#[test]
fn shared_rebuild_vector_indexes_reclaims_stale_ivf_entries() {
    let shared = SharedGraph::new(GraphId::new(8111));
    let label = db_string("vector.index.rebuild.ivf.doc");
    let property = db_string("embedding");
    let ids = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let mut ids = Vec::new();
        for row in 0..24 {
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
            VectorIndexKind::IvfSquaredEuclidean,
            2,
        )
        .unwrap();

    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        for (offset, id) in ids.iter().copied().take(4).enumerate() {
            mutator
                .update_node(
                    id,
                    LabelDiff::new([], []).unwrap(),
                    PropertyDiff::new(
                        [(property.clone(), vector(&[500.0 + offset as f32, 0.0]))],
                        [],
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        for id in ids.iter().copied().skip(4).take(2) {
            mutator.delete_node(id).unwrap();
        }
        txn.commit().unwrap();
    }

    let before = shared
        .read()
        .vector_index_for(&label, &property)
        .unwrap()
        .memory_usage();
    assert_eq!(before.indexed_rows, 22);
    assert_eq!(before.ivf_entries, 24);
    assert_eq!(before.ivf_live_entries, 22);
    assert_eq!(before.ivf_deleted_entries, 2);
    assert_eq!(before.ivf_assigned_entries, 22);
    assert_eq!(before.ivf_pending_retrain_entries, 4);

    let report = shared.rebuild_vector_indexes().unwrap();

    assert_eq!(report.indexes_rebuilt, 1);
    assert_eq!(report.reclaimed_ivf_entries, 2);
    assert_eq!(report.reclaimed_ivf_deleted_entries, 2);
    let entry = &report.entries[0];
    assert_eq!(entry.label, label);
    assert_eq!(entry.property, property);
    assert_eq!(entry.kind, VectorIndexKind::IvfSquaredEuclidean);
    assert_eq!(entry.before, before);
    assert_eq!(entry.after.indexed_rows, 22);
    assert_eq!(entry.after.ivf_entries, 22);
    assert_eq!(entry.after.ivf_live_entries, 22);
    assert_eq!(entry.after.ivf_deleted_entries, 0);
    assert_eq!(entry.after.ivf_assigned_entries, 22);
    assert_eq!(entry.after.ivf_pending_retrain_entries, 0);
    assert!(entry.after.ivf_non_empty_list_count > 0);
    assert!(entry.after.ivf_max_list_len > 0);
    assert_eq!(
        entry.after.ivf_average_list_len_basis_points,
        entry.after.ivf_assigned_entries * 10_000 / entry.after.ivf_list_count
    );

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
            &VectorValue::new(vec![500.0, 0.0]).unwrap(),
            ApproximateVectorSearchOptions::new(VectorMetric::SquaredEuclidean, 1, usize::MAX),
            CancellationChecker::disabled(),
        )
        .unwrap();
    assert_eq!(hits[0].node_id, ids[0]);
    assert_eq!(hits[0].distance, 0.0);
}
