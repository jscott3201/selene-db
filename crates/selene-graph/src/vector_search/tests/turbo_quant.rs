use selene_core::{CancellationChecker, GraphId, LabelSet, Value, VectorMetric, db_string};

use super::super::ApproximateVectorSearchOptions;
use super::{props, vector};
use crate::VectorIndexKind;
use crate::shared::SharedGraph;

#[test]
fn turbo_quant_search_reranks_primary_vectors_without_shadow_storage() {
    let shared = SharedGraph::new(GraphId::new(973));
    let doc = db_string("vector.ann.turbo.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        for components in [
            [1.0, 0.0, 0.0],
            [0.9, 0.1, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ] {
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&components))),
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }
    shared
        .create_vector_index(
            doc.clone(),
            embedding.clone(),
            VectorIndexKind::TurboQuantCosine,
            3,
        )
        .unwrap();

    let usage = shared
        .read()
        .vector_index_for(&doc, &embedding)
        .unwrap()
        .memory_usage();
    assert_eq!(usage.turbo_quant_entries, 4);
    assert_eq!(usage.turbo_quant_referenced_vector_bytes, 0);
    assert_eq!(usage.estimated_reachable_bytes, usage.estimated_index_bytes);

    let query = vector(&[1.0, 0.0, 0.0]);
    let exact = shared
        .exact_vector_search_nodes(&doc, &embedding, &query, VectorMetric::Cosine, 2)
        .unwrap();
    let approx = shared
        .approximate_vector_search_nodes_checked(
            &doc,
            &embedding,
            &query,
            ApproximateVectorSearchOptions::new(VectorMetric::Cosine, 2, 4),
            CancellationChecker::disabled(),
        )
        .unwrap();

    assert_eq!(approx, exact);
}

#[test]
fn turbo_quant_batch_search_matches_single_queries() {
    let shared = SharedGraph::new(GraphId::new(974));
    let doc = db_string("vector.ann.turbo.batch.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        for value in 0..32 {
            let angle = value as f32 * 0.05;
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(
                        &embedding,
                        Value::Vector(vector(&[angle.cos(), angle.sin()])),
                    ),
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }
    shared
        .create_vector_index(
            doc.clone(),
            embedding.clone(),
            VectorIndexKind::TurboQuantCosine,
            2,
        )
        .unwrap();

    let queries = vec![
        vector(&[1.0, 0.0]),
        vector(&[0.8, 0.6]),
        vector(&[0.25, 1.0]),
    ];
    let options = ApproximateVectorSearchOptions::new(VectorMetric::Cosine, 4, 16);
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
