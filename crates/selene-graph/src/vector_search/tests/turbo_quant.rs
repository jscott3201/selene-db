use selene_core::{CancellationChecker, GraphId, LabelSet, NodeId, Value, VectorMetric, db_string};

use super::super::{ApproximateVectorSearchOptions, VectorCandidateSet};
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

#[test]
fn turbo_quant_candidate_set_search_filters_index_rows_and_reranks() {
    let shared = SharedGraph::new(GraphId::new(975));
    let doc = db_string("vector.ann.turbo.candidates.doc").unwrap();
    let other = db_string("vector.ann.turbo.candidates.other").unwrap();
    let embedding = db_string("embedding").unwrap();
    let (doc_ids, wrong_label) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let mut doc_ids = Vec::new();
        for components in [
            [1.0, 0.0, 0.0],
            [0.95, 0.1, 0.0],
            [0.0, 1.0, 0.0],
            [0.8, 0.2, 0.0],
            [-1.0, 0.0, 0.0],
        ] {
            doc_ids.push(
                mutator
                    .create_node(
                        LabelSet::single(doc.clone()),
                        props(&embedding, Value::Vector(vector(&components))),
                    )
                    .unwrap(),
            );
        }
        let wrong_label = mutator
            .create_node(
                LabelSet::single(other),
                props(&embedding, Value::Vector(vector(&[1.0, 0.0, 0.0]))),
            )
            .unwrap();
        txn.commit().unwrap();
        (doc_ids, wrong_label)
    };
    shared
        .create_vector_index(
            doc.clone(),
            embedding.clone(),
            VectorIndexKind::TurboQuantCosine,
            3,
        )
        .unwrap();

    let candidate_set = VectorCandidateSet::from_nodes([
        doc_ids[4],
        doc_ids[3],
        doc_ids[1],
        wrong_label,
        NodeId::new(99_999),
    ]);
    let hits = shared
        .approximate_vector_search_candidate_set_checked(
            &doc,
            &embedding,
            &vector(&[1.0, 0.0, 0.0]),
            &candidate_set,
            ApproximateVectorSearchOptions::new(VectorMetric::Cosine, 2, 8),
            CancellationChecker::disabled(),
        )
        .unwrap();

    assert_eq!(
        hits.iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![doc_ids[1], doc_ids[3]]
    );
    assert!(!hits.iter().any(|hit| hit.node_id == wrong_label));
}
