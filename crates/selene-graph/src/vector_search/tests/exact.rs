use super::*;

#[test]
fn exact_vector_search_ranks_labelled_vector_nodes() {
    let shared = SharedGraph::new(GraphId::new(91));
    let doc = db_string("vector.doc").unwrap();
    let other = db_string("vector.other").unwrap();
    let embedding = db_string("embedding").unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[2.0, 0.0]))),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[1.0, 0.0]))),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::String(db_string("skip").unwrap())),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(other),
                props(&embedding, Value::Vector(vector(&[0.0, 0.0]))),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let hits = shared
        .exact_vector_search_nodes(
            &doc,
            &embedding,
            &vector(&[0.0, 0.0]),
            VectorMetric::SquaredEuclidean,
            10,
        )
        .unwrap();

    assert_eq!(
        hits,
        vec![
            VectorNodeSearchHit {
                node_id: NodeId::new(2),
                distance: 1.0,
            },
            VectorNodeSearchHit {
                node_id: NodeId::new(1),
                distance: 4.0,
            },
        ]
    );
}

#[test]
fn exact_vector_search_zero_k_and_missing_label_are_empty() {
    let shared = SharedGraph::new(GraphId::new(92));
    let doc = db_string("vector.empty.doc").unwrap();
    let missing = db_string("vector.empty.missing").unwrap();
    let embedding = db_string("embedding").unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[1.0]))),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    assert!(
        shared
            .exact_vector_search_nodes(&doc, &embedding, &vector(&[0.0]), VectorMetric::Cosine, 0,)
            .unwrap()
            .is_empty()
    );
    assert!(
        shared
            .exact_vector_search_nodes(
                &missing,
                &embedding,
                &vector(&[0.0]),
                VectorMetric::SquaredEuclidean,
                10,
            )
            .unwrap()
            .is_empty()
    );
}

#[test]
fn exact_vector_search_checked_observes_cancelled_token_before_scan() {
    let shared = SharedGraph::new(GraphId::new(93));
    let doc = db_string("vector.cancel.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[1.0]))),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let token = CancellationToken::new();
    token.cancel();
    let checker = CancellationChecker::new(Some(&token), None);
    let err = shared
        .exact_vector_search_nodes_checked(
            &doc,
            &embedding,
            &vector(&[0.0]),
            VectorMetric::SquaredEuclidean,
            10,
            checker,
        )
        .unwrap_err();

    assert!(matches!(err, VectorSearchError::Cancelled));
}

#[test]
fn exact_vector_search_checked_observes_elapsed_deadline_before_scan() {
    let shared = SharedGraph::new(GraphId::new(94));
    let doc = db_string("vector.timeout.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[1.0]))),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let checker = CancellationChecker::new(None, Some(Instant::now() - Duration::from_millis(1)));
    let err = shared
        .exact_vector_search_nodes_checked(
            &doc,
            &embedding,
            &vector(&[0.0]),
            VectorMetric::SquaredEuclidean,
            10,
            checker,
        )
        .unwrap_err();

    assert!(matches!(err, VectorSearchError::Timeout { .. }));
}

#[test]
fn exact_vector_search_uses_node_id_tie_breaks() {
    let shared = SharedGraph::new(GraphId::new(93));
    let doc = db_string("vector.tie.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        for _ in 0..3 {
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[1.0]))),
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }

    let hits = shared
        .exact_vector_search_nodes(
            &doc,
            &embedding,
            &vector(&[0.0]),
            VectorMetric::SquaredEuclidean,
            2,
        )
        .unwrap();

    assert_eq!(
        hits,
        vec![
            VectorNodeSearchHit {
                node_id: NodeId::new(1),
                distance: 1.0,
            },
            VectorNodeSearchHit {
                node_id: NodeId::new(2),
                distance: 1.0,
            },
        ]
    );
}

#[test]
fn exact_vector_search_checked_matches_disabled_ordering() {
    let shared = SharedGraph::new(GraphId::new(9301));
    let doc = db_string("vector.parallel.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        for idx in 0..16 {
            let value = ((idx * 7) % 5) as f32;
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[value]))),
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }

    let query = vector(&[0.0]);
    let token = CancellationToken::new();
    let checked = shared
        .exact_vector_search_nodes_checked(
            &doc,
            &embedding,
            &query,
            VectorMetric::SquaredEuclidean,
            7,
            CancellationChecker::new(Some(&token), None),
        )
        .unwrap();
    let unindexed_disabled = shared
        .exact_vector_search_nodes(&doc, &embedding, &query, VectorMetric::SquaredEuclidean, 7)
        .unwrap();
    assert_eq!(unindexed_disabled, checked);

    shared
        .create_vector_index(doc.clone(), embedding.clone(), VectorIndexKind::Flat, 1)
        .unwrap();

    let indexed = shared
        .exact_vector_search_nodes(&doc, &embedding, &query, VectorMetric::SquaredEuclidean, 7)
        .unwrap();

    assert_eq!(indexed, checked);
}

#[test]
fn exact_vector_search_tracks_update_and_delete_visibility() {
    let shared = SharedGraph::new(GraphId::new(94));
    let doc = db_string("vector.visible.doc").unwrap();
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
        .exact_vector_search_nodes(
            &doc,
            &embedding,
            &vector(&[0.0]),
            VectorMetric::SquaredEuclidean,
            10,
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
fn exact_vector_search_surfaces_metric_errors() {
    let shared = SharedGraph::new(GraphId::new(95));
    let doc = db_string("vector.error.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[1.0, 2.0]))),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let error = shared
        .exact_vector_search_nodes(
            &doc,
            &embedding,
            &vector(&[1.0]),
            VectorMetric::SquaredEuclidean,
            10,
        )
        .unwrap_err();

    assert!(matches!(
        error,
        GraphError::Core(CoreError::VectorDimensionMismatch { lhs: 1, rhs: 2 })
    ));
}

#[test]
fn exact_vector_search_surfaces_cosine_zero_norm() {
    let shared = SharedGraph::new(GraphId::new(96));
    let doc = db_string("vector.cosine.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[0.0, 0.0]))),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let error = shared
        .exact_vector_search_nodes(
            &doc,
            &embedding,
            &vector(&[1.0, 0.0]),
            VectorMetric::Cosine,
            10,
        )
        .unwrap_err();

    assert!(matches!(
        error,
        GraphError::Core(CoreError::VectorZeroNorm { side: "rhs" })
    ));
}
