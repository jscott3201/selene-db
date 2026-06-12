use selene_core::{CancellationChecker, GraphId, LabelSet, Value, VectorMetric, db_string};

use super::super::ApproximateVectorSearchOptions;
use super::{props, vector};
use crate::VectorIndexKind;
use crate::shared::SharedGraph;

#[test]
fn approximate_vector_search_batch_uses_ivf_index() {
    let shared = SharedGraph::new(GraphId::new(975));
    let doc = db_string("vector.ann.ivf.batch.doc").unwrap();
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

    let queries = vec![
        vector(&[4.1, 0.0]),
        vector(&[31.7, 0.0]),
        vector(&[47.3, 0.0]),
    ];
    let options =
        ApproximateVectorSearchOptions::new(VectorMetric::SquaredEuclidean, 4, usize::MAX);
    let batched = shared
        .approximate_vector_search_nodes_batch_checked(
            &doc,
            &embedding,
            &queries,
            options,
            CancellationChecker::disabled(),
        )
        .unwrap();
    let singles = queries
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
        .collect::<Vec<_>>();

    assert_eq!(batched, singles);
}
