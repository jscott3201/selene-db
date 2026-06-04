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
