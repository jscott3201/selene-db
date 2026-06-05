use selene_core::{
    CancellationChecker, GraphId, LabelSet, NodeId, PropertyMap, Value, VectorMetric, VectorValue,
    intern,
};

use crate::{
    ApproximateVectorExpansionOptions, SharedGraph, VectorIndexKind, VectorNeighborDirection,
};

fn vector(components: &[f32]) -> VectorValue {
    VectorValue::new(components.to_vec()).expect("test vector is valid")
}

fn props(key: &selene_core::IStr, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
}

#[test]
fn approximate_vector_search_expanded_candidates_uses_ann_roots_then_graph_rerank() {
    let shared = SharedGraph::new(GraphId::new(9891));
    let summary = intern("vector.ann.expand.summary").unwrap();
    let fact = intern("vector.ann.expand.fact").unwrap();
    let embedding = intern("embedding").unwrap();
    let supports = intern("SUPPORTS").unwrap();
    let mentions = intern("MENTIONS").unwrap();
    let (root, supported, wrong_label) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let root = mutator
            .create_node(
                LabelSet::single(summary.clone()),
                props(&embedding, Value::Vector(vector(&[0.2, 0.0]))),
            )
            .unwrap();
        let supported = mutator
            .create_node(
                LabelSet::single(fact.clone()),
                props(&embedding, Value::Vector(vector(&[0.0, 0.0]))),
            )
            .unwrap();
        let wrong_label = mutator
            .create_node(
                LabelSet::single(fact),
                props(&embedding, Value::Vector(vector(&[0.0, 0.0]))),
            )
            .unwrap();
        mutator
            .create_edge(supports.clone(), root, supported, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(mentions, root, wrong_label, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        (root, supported, wrong_label)
    };
    shared
        .create_vector_index(
            summary.clone(),
            embedding.clone(),
            VectorIndexKind::HnswSquaredEuclidean,
            2,
        )
        .unwrap();

    let query = vector(&[0.0, 0.0]);
    let roots = shared
        .approximate_vector_search_nodes_checked(
            &summary,
            &embedding,
            &query,
            crate::ApproximateVectorSearchOptions::new(VectorMetric::SquaredEuclidean, 1, 16),
            CancellationChecker::disabled(),
        )
        .unwrap();
    let expanded = shared
        .approximate_vector_search_expanded_candidates_checked(
            &summary,
            &embedding,
            &query,
            ApproximateVectorExpansionOptions::new(
                &supports,
                VectorNeighborDirection::Outgoing,
                VectorMetric::SquaredEuclidean,
                1,
                2,
                16,
            ),
            CancellationChecker::disabled(),
        )
        .unwrap();

    assert_eq!(
        roots.iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![root]
    );
    assert_eq!(
        expanded.iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![supported, root]
    );
    assert!(!expanded.iter().any(|hit| hit.node_id == wrong_label));
}

#[test]
fn approximate_vector_search_expanded_candidates_batch_matches_single_queries() {
    let shared = SharedGraph::new(GraphId::new(9892));
    let summary = intern("vector.ann.expand.batch.summary").unwrap();
    let fact = intern("vector.ann.expand.batch.fact").unwrap();
    let embedding = intern("embedding").unwrap();
    let supports = intern("SUPPORTS").unwrap();
    let (root_a, fact_a, root_b, fact_b) =
        create_two_root_expansion_graph(&shared, &summary, &fact, &embedding, &supports);
    shared
        .create_vector_index(
            summary.clone(),
            embedding.clone(),
            VectorIndexKind::HnswSquaredEuclidean,
            2,
        )
        .unwrap();

    let queries = vec![vector(&[0.0, 0.0]), vector(&[10.0, 0.0])];
    let options = ApproximateVectorExpansionOptions::new(
        &supports,
        VectorNeighborDirection::Outgoing,
        VectorMetric::SquaredEuclidean,
        2,
        1,
        32,
    );
    let batched = shared
        .approximate_vector_search_expanded_candidates_batch_checked(
            &summary,
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
                .approximate_vector_search_expanded_candidates_checked(
                    &summary,
                    &embedding,
                    query,
                    options,
                    CancellationChecker::disabled(),
                )
                .unwrap()
        })
        .collect::<Vec<_>>();

    assert_eq!(batched, singles);
    assert_eq!(node_ids(&batched[0]), vec![fact_a]);
    assert_eq!(node_ids(&batched[1]), vec![fact_b]);
    assert!(!node_ids(&batched[0]).contains(&root_b));
    assert!(!node_ids(&batched[1]).contains(&root_a));
}

#[test]
fn approximate_vector_search_expanded_candidates_preserves_ann_errors() {
    let shared = SharedGraph::new(GraphId::new(9893));
    let summary = intern("vector.ann.expand.error.summary").unwrap();
    let embedding = intern("embedding").unwrap();
    let supports = intern("SUPPORTS").unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(summary.clone()),
                props(&embedding, Value::Vector(vector(&[1.0, 0.0]))),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let err = shared
        .approximate_vector_search_expanded_candidates_checked(
            &summary,
            &embedding,
            &vector(&[1.0, 0.0]),
            ApproximateVectorExpansionOptions::new(
                &supports,
                VectorNeighborDirection::Outgoing,
                VectorMetric::SquaredEuclidean,
                1,
                1,
                16,
            ),
            CancellationChecker::disabled(),
        )
        .expect_err("ANN-root graph expansion requires a matching ANN index");

    assert!(matches!(
        err,
        crate::VectorSearchError::ApproximateIndexMissing
    ));
}

fn create_two_root_expansion_graph(
    shared: &SharedGraph,
    summary: &selene_core::IStr,
    fact: &selene_core::IStr,
    embedding: &selene_core::IStr,
    supports: &selene_core::IStr,
) -> (NodeId, NodeId, NodeId, NodeId) {
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let root_a = mutator
        .create_node(
            LabelSet::single(summary.clone()),
            props(embedding, Value::Vector(vector(&[0.2, 0.0]))),
        )
        .unwrap();
    let fact_a = mutator
        .create_node(
            LabelSet::single(fact.clone()),
            props(embedding, Value::Vector(vector(&[0.0, 0.0]))),
        )
        .unwrap();
    let root_b = mutator
        .create_node(
            LabelSet::single(summary.clone()),
            props(embedding, Value::Vector(vector(&[10.2, 0.0]))),
        )
        .unwrap();
    let fact_b = mutator
        .create_node(
            LabelSet::single(fact.clone()),
            props(embedding, Value::Vector(vector(&[10.0, 0.0]))),
        )
        .unwrap();
    mutator
        .create_edge(supports.clone(), root_a, fact_a, PropertyMap::new())
        .unwrap();
    mutator
        .create_edge(supports.clone(), root_b, fact_b, PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();
    (root_a, fact_a, root_b, fact_b)
}

fn node_ids(hits: &[crate::VectorNodeSearchHit]) -> Vec<NodeId> {
    hits.iter().map(|hit| hit.node_id).collect()
}
