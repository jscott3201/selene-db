use selene_core::{
    CancellationChecker, CoreError, GraphId, LabelSet, PropertyMap, Value, VectorMetric,
    VectorValue, intern,
};

use crate::{GraphError, SharedGraph, VectorIndexKind, VectorSearchError};

fn vector(components: &[f32]) -> VectorValue {
    VectorValue::new(components.to_vec()).expect("test vector is valid")
}

fn props(key: &selene_core::IStr, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
}

#[test]
fn exact_vector_search_batch_matches_single_queries() {
    let shared = SharedGraph::new(GraphId::new(972));
    let doc = intern("vector.exact.batch.doc").unwrap();
    let embedding = intern("embedding").unwrap();
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
        .create_vector_index(doc.clone(), embedding.clone(), VectorIndexKind::Flat, 2)
        .unwrap();

    let queries = vec![
        vector(&[4.1, 0.0]),
        vector(&[31.7, 0.0]),
        vector(&[74.3, 0.0]),
    ];
    let batched = shared
        .exact_vector_search_nodes_batch_checked(
            &doc,
            &embedding,
            &queries,
            VectorMetric::SquaredEuclidean,
            4,
            CancellationChecker::disabled(),
        )
        .unwrap();
    let singles: Vec<_> = queries
        .iter()
        .map(|query| {
            shared
                .exact_vector_search_nodes_checked(
                    &doc,
                    &embedding,
                    query,
                    VectorMetric::SquaredEuclidean,
                    4,
                    CancellationChecker::disabled(),
                )
                .unwrap()
        })
        .collect();

    assert_eq!(batched, singles);
}

#[test]
fn exact_vector_search_batch_rejects_mixed_query_dimensions() {
    let shared = SharedGraph::new(GraphId::new(973));
    let doc = intern("vector.exact.batch.mixed.doc").unwrap();
    let embedding = intern("embedding").unwrap();
    let queries = vec![vector(&[0.0, 0.0]), vector(&[0.0, 0.0, 0.0])];

    let err = shared
        .exact_vector_search_nodes_batch_checked(
            &doc,
            &embedding,
            &queries,
            VectorMetric::SquaredEuclidean,
            4,
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
