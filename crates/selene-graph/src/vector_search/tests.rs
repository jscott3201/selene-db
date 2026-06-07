use std::time::{Duration, Instant};

use selene_core::{
    CancellationChecker, CancellationToken, CoreError, GraphId, LabelDiff, LabelSet, PropertyDiff,
    PropertyMap, Value, VectorValue, db_string,
};

use super::*;
use crate::VectorIndexKind;

fn vector(components: &[f32]) -> VectorValue {
    VectorValue::new(components.to_vec()).expect("test vector is valid")
}

fn props(key: &DbString, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
}

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
fn exact_vector_search_parallel_paths_match_sequential_ordering() {
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
    let sequential = shared
        .exact_vector_search_nodes_checked(
            &doc,
            &embedding,
            &query,
            VectorMetric::SquaredEuclidean,
            7,
            CancellationChecker::new(Some(&token), None),
        )
        .unwrap();
    let unindexed_parallel = shared
        .exact_vector_search_nodes(&doc, &embedding, &query, VectorMetric::SquaredEuclidean, 7)
        .unwrap();
    assert_eq!(unindexed_parallel, sequential);

    shared
        .create_vector_index(doc.clone(), embedding.clone(), VectorIndexKind::Flat, 1)
        .unwrap();

    let indexed = shared
        .exact_vector_search_nodes(&doc, &embedding, &query, VectorMetric::SquaredEuclidean, 7)
        .unwrap();

    assert_eq!(indexed, sequential);
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
