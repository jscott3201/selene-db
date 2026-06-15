use super::*;

#[test]
fn hnsw_vector_index_tracks_membership_and_metric() {
    let shared = SharedGraph::new(GraphId::new(8105));
    let label = db_string("vector.index.hnsw");
    let property = db_string("embedding");
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
fn ivf_vector_index_tracks_membership_and_metric() {
    let shared = SharedGraph::new(GraphId::new(8110));
    let label = db_string("vector.index.ivf");
    let property = db_string("embedding");
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
            VectorIndexKind::IvfSquaredEuclidean,
            2,
        )
        .unwrap();

    let index = shared.read().vector_index_for(&label, &property).unwrap();
    assert!(index.is_ivf());
    assert_eq!(index.ann_metric(), Some(VectorMetric::SquaredEuclidean));
    assert_eq!(index.rows().iter().collect::<Vec<_>>(), vec![0, 1]);
    let usage = index.memory_usage();
    assert_eq!(usage.ivf_entries, 2);
    assert_eq!(usage.ivf_live_entries, 2);
    assert_eq!(usage.ivf_assigned_entries, 2);
    assert_eq!(usage.ivf_pending_retrain_entries, 0);
    assert!(usage.ivf_non_empty_list_count > 0);
    assert!(usage.ivf_max_list_len > 0);
    assert_eq!(
        usage.ivf_average_list_len_basis_points,
        usage.ivf_assigned_entries * 10_000 / usage.ivf_list_count
    );
    assert!(usage.ivf_centroids > 0);
}

#[test]
fn hnsw_vector_index_stores_explicit_construction_config() {
    let shared = SharedGraph::new(GraphId::new(8108));
    let label = db_string("vector.index.hnsw.config");
    let property = db_string("embedding");
    let config = HnswIndexConfig::new(24, 128);
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
                props([(property.clone(), vector(&[0.0, 1.0]))]),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    shared
        .create_vector_index_named_with_config(
            label.clone(),
            property.clone(),
            VectorIndexKind::HnswCosine,
            2,
            None,
            Some(config),
        )
        .unwrap();

    let index = shared.read().vector_index_for(&label, &property).unwrap();
    assert_eq!(index.hnsw_config(), Some(config));
    assert_eq!(index.hnsw_metric(), Some(VectorMetric::Cosine));
    assert_eq!(index.rows().iter().collect::<Vec<_>>(), vec![0, 1]);
}

#[test]
fn hnsw_config_is_rejected_for_flat_or_invalid_hnsw_shapes() {
    let shared = SharedGraph::new(GraphId::new(8109));
    let label = db_string("vector.index.hnsw.config.reject");
    let property = db_string("embedding");

    let flat_err = shared
        .create_vector_index_named_with_config(
            label.clone(),
            property.clone(),
            VectorIndexKind::Flat,
            2,
            None,
            Some(HnswIndexConfig::new(24, 128)),
        )
        .unwrap_err();
    assert!(matches!(
        flat_err,
        GraphError::VectorIndexInvalidHnswConfig {
            max_neighbors: 24,
            ef_construction: 128,
            reason: "only HNSW vector indexes accept HNSW config",
        }
    ));

    let invalid_err = shared
        .create_vector_index_named_with_config(
            label,
            property,
            VectorIndexKind::HnswSquaredEuclidean,
            2,
            None,
            Some(HnswIndexConfig::new(24, 8)),
        )
        .unwrap_err();
    assert!(matches!(
        invalid_err,
        GraphError::VectorIndexInvalidHnswConfig {
            max_neighbors: 24,
            ef_construction: 8,
            reason: "ef_construction must be at least max_neighbors",
        }
    ));
}
