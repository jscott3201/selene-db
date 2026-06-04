use selene_core::{
    CancellationChecker, CoreError, GraphId, LabelSet, NodeId, PropertyMap, Value, VectorMetric,
    VectorValue, intern,
};

use crate::{GraphError, SharedGraph, VectorSearchError};

fn vector(components: &[f32]) -> VectorValue {
    VectorValue::new(components.to_vec()).expect("test vector is valid")
}

fn props(key: &selene_core::IStr, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
}

#[test]
fn score_vector_nodes_ranks_unique_live_vector_candidates() {
    let shared = SharedGraph::new(GraphId::new(974));
    let label = intern("vector.score.doc").unwrap();
    let embedding = intern("embedding").unwrap();
    let other = intern("other").unwrap();
    let ids = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let mut ids = Vec::new();
        for value in 0..5 {
            ids.push(
                mutator
                    .create_node(
                        LabelSet::single(label.clone()),
                        props(&embedding, Value::Vector(vector(&[value as f32, 0.0]))),
                    )
                    .unwrap(),
            );
        }
        ids.push(
            mutator
                .create_node(
                    LabelSet::single(label.clone()),
                    props(&other, Value::String(intern("not-a-vector").unwrap())),
                )
                .unwrap(),
        );
        txn.commit().unwrap();
        ids
    };
    {
        let mut txn = shared.begin_write();
        txn.mutator().delete_node(ids[4]).unwrap();
        txn.commit().unwrap();
    }

    let hits = shared
        .score_vector_nodes_checked(
            &embedding,
            &vector(&[2.1, 0.0]),
            &[
                ids[3],
                ids[2],
                ids[2],
                ids[0],
                ids[4],
                ids[5],
                NodeId::new(999),
            ],
            VectorMetric::SquaredEuclidean,
            3,
            CancellationChecker::disabled(),
        )
        .unwrap();

    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].node_id, ids[2]);
    assert_eq!(hits[1].node_id, ids[3]);
    assert_eq!(hits[2].node_id, ids[0]);
}

#[test]
fn score_vector_nodes_zero_k_does_not_bind_query() {
    let shared = SharedGraph::new(GraphId::new(975));
    let embedding = intern("embedding").unwrap();

    let hits = shared
        .score_vector_nodes_checked(
            &embedding,
            &vector(&[0.0, 0.0]),
            &[NodeId::new(1)],
            VectorMetric::Cosine,
            0,
            CancellationChecker::disabled(),
        )
        .unwrap();

    assert!(hits.is_empty());
}

#[test]
fn score_vector_nodes_surfaces_candidate_dimension_errors() {
    let shared = SharedGraph::new(GraphId::new(976));
    let label = intern("vector.score.dim.doc").unwrap();
    let embedding = intern("embedding").unwrap();
    let node = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let node = mutator
            .create_node(
                LabelSet::single(label),
                props(&embedding, Value::Vector(vector(&[1.0, 2.0, 3.0]))),
            )
            .unwrap();
        txn.commit().unwrap();
        node
    };

    let err = shared
        .score_vector_nodes_checked(
            &embedding,
            &vector(&[1.0, 2.0]),
            &[node],
            VectorMetric::SquaredEuclidean,
            10,
            CancellationChecker::disabled(),
        )
        .expect_err("dimension mismatch must error");

    assert!(matches!(
        err,
        VectorSearchError::Graph(GraphError::Core(CoreError::VectorDimensionMismatch {
            lhs: 2,
            rhs: 3
        }))
    ));
}

#[test]
fn score_vector_nodes_batch_matches_single_queries() {
    let shared = SharedGraph::new(GraphId::new(977));
    let label = intern("vector.score.batch.doc").unwrap();
    let embedding = intern("embedding").unwrap();
    let ids = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let mut ids = Vec::new();
        for value in 0..8 {
            ids.push(
                mutator
                    .create_node(
                        LabelSet::single(label.clone()),
                        props(&embedding, Value::Vector(vector(&[value as f32, 0.0]))),
                    )
                    .unwrap(),
            );
        }
        txn.commit().unwrap();
        ids
    };
    let queries = vec![vector(&[2.2, 0.0]), vector(&[5.1, 0.0])];
    let candidate_sets = vec![
        vec![ids[4], ids[2], ids[2], ids[0], NodeId::new(999)],
        vec![ids[7], ids[5], ids[1], ids[5]],
    ];

    let batched = shared
        .score_vector_nodes_batch_checked(
            &embedding,
            &queries,
            &candidate_sets,
            VectorMetric::SquaredEuclidean,
            2,
            CancellationChecker::disabled(),
        )
        .unwrap();
    let singles: Vec<_> = queries
        .iter()
        .zip(&candidate_sets)
        .map(|(query, candidates)| {
            shared
                .score_vector_nodes_checked(
                    &embedding,
                    query,
                    candidates,
                    VectorMetric::SquaredEuclidean,
                    2,
                    CancellationChecker::disabled(),
                )
                .unwrap()
        })
        .collect();

    assert_eq!(batched, singles);
    assert_eq!(batched[0][0].node_id, ids[2]);
    assert_eq!(batched[1][0].node_id, ids[5]);
}

#[test]
fn score_vector_nodes_batch_empty_and_zero_k_do_not_bind_queries() {
    let shared = SharedGraph::new(GraphId::new(978));
    let embedding = intern("embedding").unwrap();

    let empty = shared
        .score_vector_nodes_batch_checked::<Vec<NodeId>>(
            &embedding,
            &[],
            &[],
            VectorMetric::Cosine,
            10,
            CancellationChecker::disabled(),
        )
        .unwrap();
    assert!(empty.is_empty());

    let hits = shared
        .score_vector_nodes_batch_checked(
            &embedding,
            &[vector(&[0.0, 0.0]), vector(&[0.0, 0.0])],
            &[vec![NodeId::new(1)], vec![NodeId::new(2)]],
            VectorMetric::Cosine,
            0,
            CancellationChecker::disabled(),
        )
        .unwrap();
    assert_eq!(hits, vec![Vec::new(), Vec::new()]);
}

#[test]
fn score_vector_nodes_batch_rejects_invalid_batch_shape() {
    let shared = SharedGraph::new(GraphId::new(979));
    let embedding = intern("embedding").unwrap();

    let err = shared
        .score_vector_nodes_batch_checked(
            &embedding,
            &[vector(&[0.0, 0.0]), vector(&[1.0, 0.0])],
            &[vec![NodeId::new(1)]],
            VectorMetric::SquaredEuclidean,
            1,
            CancellationChecker::disabled(),
        )
        .expect_err("query/candidate batch arity mismatch must error");
    assert!(matches!(
        err,
        VectorSearchError::BatchLengthMismatch {
            queries: 2,
            candidate_sets: 1
        }
    ));

    let err = shared
        .score_vector_nodes_batch_checked(
            &embedding,
            &[vector(&[0.0, 0.0]), vector(&[1.0, 0.0, 0.0])],
            &[vec![NodeId::new(1)], vec![NodeId::new(2)]],
            VectorMetric::SquaredEuclidean,
            1,
            CancellationChecker::disabled(),
        )
        .expect_err("mixed query dimensions must error");
    assert!(matches!(
        err,
        VectorSearchError::Graph(GraphError::Core(CoreError::VectorDimensionMismatch {
            lhs: 2,
            rhs: 3
        }))
    ));
}
