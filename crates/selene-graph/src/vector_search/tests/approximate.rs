use super::*;

#[test]
fn approximate_vector_search_uses_hnsw_index() {
    let shared = SharedGraph::new(GraphId::new(97));
    let doc = db_string("vector.ann.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        for value in 0..64 {
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[value as f32, 0.0]))),
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }
    shared
        .create_vector_index(
            doc.clone(),
            embedding.clone(),
            VectorIndexKind::HnswSquaredEuclidean,
            2,
        )
        .unwrap();

    let query = vector(&[4.1, 0.0]);
    let exact = shared
        .exact_vector_search_nodes(&doc, &embedding, &query, VectorMetric::SquaredEuclidean, 3)
        .unwrap();
    let approx = shared
        .approximate_vector_search_nodes_checked(
            &doc,
            &embedding,
            &query,
            ApproximateVectorSearchOptions::new(VectorMetric::SquaredEuclidean, 3, 32),
            CancellationChecker::disabled(),
        )
        .unwrap();

    assert_eq!(approx[0], exact[0]);
    assert_eq!(approx.len(), 3);
}

#[test]
fn approximate_vector_search_uses_ivf_index() {
    let shared = SharedGraph::new(GraphId::new(972));
    let doc = db_string("vector.ann.ivf.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        for value in 0..64 {
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[value as f32, 0.0]))),
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }
    shared
        .create_vector_index(
            doc.clone(),
            embedding.clone(),
            VectorIndexKind::IvfSquaredEuclidean,
            2,
        )
        .unwrap();

    let query = vector(&[4.1, 0.0]);
    let exact = shared
        .exact_vector_search_nodes(&doc, &embedding, &query, VectorMetric::SquaredEuclidean, 3)
        .unwrap();
    let approx = shared
        .approximate_vector_search_nodes_checked(
            &doc,
            &embedding,
            &query,
            ApproximateVectorSearchOptions::new(VectorMetric::SquaredEuclidean, 3, usize::MAX),
            CancellationChecker::disabled(),
        )
        .unwrap();

    assert_eq!(approx, exact);
}

#[test]
fn approximate_vector_search_batch_matches_single_queries() {
    let shared = SharedGraph::new(GraphId::new(971));
    let doc = db_string("vector.ann.batch.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        for value in 0..96 {
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[value as f32, 0.0]))),
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }
    shared
        .create_vector_index(
            doc.clone(),
            embedding.clone(),
            VectorIndexKind::HnswSquaredEuclidean,
            2,
        )
        .unwrap();

    let queries = vec![
        vector(&[4.1, 0.0]),
        vector(&[31.7, 0.0]),
        vector(&[74.3, 0.0]),
    ];
    let options = ApproximateVectorSearchOptions::new(VectorMetric::SquaredEuclidean, 4, 32);
    let batched = shared
        .approximate_vector_search_nodes_batch_checked(
            &doc,
            &embedding,
            &queries,
            options,
            CancellationChecker::disabled(),
        )
        .unwrap();
    let singles: Vec<_> = queries
        .iter()
        .map(|query| {
            shared
                .approximate_vector_search_nodes_checked(
                    &doc,
                    &embedding,
                    query,
                    options,
                    CancellationChecker::disabled(),
                )
                .unwrap()
        })
        .collect();

    assert_eq!(batched, singles);
}

#[test]
fn approximate_vector_search_tracks_update_and_delete_visibility() {
    let shared = SharedGraph::new(GraphId::new(99));
    let doc = db_string("vector.ann.visible.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    let (first, second) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let first = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[10.0]))),
            )
            .unwrap();
        let second = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[1.0]))),
            )
            .unwrap();
        txn.commit().unwrap();
        (first, second)
    };
    shared
        .create_vector_index(
            doc.clone(),
            embedding.clone(),
            VectorIndexKind::HnswSquaredEuclidean,
            1,
        )
        .unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .update_node(
                first,
                LabelDiff::new([], []).unwrap(),
                PropertyDiff::new([(embedding.clone(), Value::Vector(vector(&[0.25])))], [])
                    .unwrap(),
            )
            .unwrap();
        mutator.delete_node(second).unwrap();
        txn.commit().unwrap();
    }

    let hits = shared
        .approximate_vector_search_nodes_checked(
            &doc,
            &embedding,
            &vector(&[0.0]),
            ApproximateVectorSearchOptions::new(VectorMetric::SquaredEuclidean, 10, 32),
            CancellationChecker::disabled(),
        )
        .unwrap();

    assert_eq!(
        hits,
        vec![VectorNodeSearchHit {
            node_id: first,
            distance: 0.0625,
        }]
    );
}

#[test]
fn approximate_vector_search_meets_recall_floor_against_exact_oracle() {
    const K: usize = 8;
    const QUERY_VALUES: &[f32] = &[3.25, 31.4, 72.8, 127.1, 180.6, 244.2];

    let shared = SharedGraph::new(GraphId::new(9701));
    let doc = db_string("vector.ann.recall.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        for idx in 0..256 {
            let jitter = ((idx * 17) % 23) as f32 / 1_000.0;
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[idx as f32, jitter]))),
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }
    shared
        .create_vector_index(
            doc.clone(),
            embedding.clone(),
            VectorIndexKind::HnswSquaredEuclidean,
            2,
        )
        .unwrap();

    let mut overlap = 0usize;
    let mut expected = 0usize;
    for &value in QUERY_VALUES {
        let query = vector(&[value, 0.0]);
        let exact = shared
            .exact_vector_search_nodes(&doc, &embedding, &query, VectorMetric::SquaredEuclidean, K)
            .unwrap();
        let approximate = shared
            .approximate_vector_search_nodes_checked(
                &doc,
                &embedding,
                &query,
                ApproximateVectorSearchOptions::new(VectorMetric::SquaredEuclidean, K, 64),
                CancellationChecker::disabled(),
            )
            .unwrap();
        expected += exact.len();
        overlap += exact
            .iter()
            .filter(|exact| {
                approximate
                    .iter()
                    .any(|approximate| approximate.node_id == exact.node_id)
            })
            .count();
    }

    assert!(
        overlap * 100 >= expected * 95,
        "HNSW recall {overlap}/{expected} fell below 95%"
    );
}

#[test]
fn approximate_vector_search_rejects_missing_or_wrong_metric_index() {
    let shared = SharedGraph::new(GraphId::new(98));
    let doc = db_string("vector.ann.missing.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[1.0, 0.0]))),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let missing = shared
        .approximate_vector_search_nodes_checked(
            &doc,
            &embedding,
            &vector(&[1.0, 0.0]),
            ApproximateVectorSearchOptions::new(VectorMetric::SquaredEuclidean, 1, 16),
            CancellationChecker::disabled(),
        )
        .unwrap_err();
    assert!(matches!(
        missing,
        VectorSearchError::ApproximateIndexMissing
    ));

    shared
        .create_vector_index(
            doc.clone(),
            embedding.clone(),
            VectorIndexKind::HnswSquaredEuclidean,
            2,
        )
        .unwrap();
    let wrong_metric = shared
        .approximate_vector_search_nodes_checked(
            &doc,
            &embedding,
            &vector(&[1.0, 0.0]),
            ApproximateVectorSearchOptions::new(VectorMetric::NegativeInnerProduct, 1, 16),
            CancellationChecker::disabled(),
        )
        .unwrap_err();
    assert!(matches!(
        wrong_metric,
        VectorSearchError::ApproximateMetricMismatch {
            indexed: VectorMetric::SquaredEuclidean,
            requested: VectorMetric::NegativeInnerProduct,
        }
    ));
}
