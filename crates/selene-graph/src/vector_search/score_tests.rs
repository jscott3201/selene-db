use selene_core::{
    CancellationChecker, CancellationToken, CoreError, GraphId, LabelSet, NodeId, PropertyMap,
    Value, VectorMetric, VectorValue, db_string,
};

use crate::{
    GraphError, SharedGraph, VectorCandidateSet, VectorNeighborDirection,
    VectorNeighborSearchOptions, VectorSearchError,
};

#[path = "score_tests/candidate_set.rs"]
mod candidate_set;

fn vector(components: &[f32]) -> VectorValue {
    VectorValue::new(components.to_vec()).expect("test vector is valid")
}

fn props(key: &selene_core::DbString, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
}

#[test]
fn score_vector_nodes_ranks_unique_live_vector_candidates() {
    let shared = SharedGraph::new(GraphId::new(974));
    let label = db_string("vector.score.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    let other = db_string("other").unwrap();
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
                    props(&other, Value::String(db_string("not-a-vector").unwrap())),
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
    let embedding = db_string("embedding").unwrap();

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
    let label = db_string("vector.score.dim.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
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
    let label = db_string("vector.score.batch.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
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
    let embedding = db_string("embedding").unwrap();

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
    let embedding = db_string("embedding").unwrap();

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

#[test]
fn score_vector_neighbors_filters_direction_and_ranks_unique_live_vectors() {
    let shared = SharedGraph::new(GraphId::new(980));
    let anchor_label = db_string("vector.neighbor.anchor").unwrap();
    let doc_label = db_string("vector.neighbor.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    let link = db_string("DEPENDS_ON").unwrap();
    let other_link = db_string("MENTIONS").unwrap();
    let other = db_string("other").unwrap();
    let (anchor, out_near, out_far, incoming, deleted, non_vector) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let anchor = mutator
            .create_node(LabelSet::single(anchor_label), PropertyMap::new())
            .unwrap();
        let out_near = mutator
            .create_node(
                LabelSet::single(doc_label.clone()),
                props(&embedding, Value::Vector(vector(&[2.0, 0.0]))),
            )
            .unwrap();
        let out_far = mutator
            .create_node(
                LabelSet::single(doc_label.clone()),
                props(&embedding, Value::Vector(vector(&[8.0, 0.0]))),
            )
            .unwrap();
        let incoming = mutator
            .create_node(
                LabelSet::single(doc_label.clone()),
                props(&embedding, Value::Vector(vector(&[1.0, 0.0]))),
            )
            .unwrap();
        let deleted = mutator
            .create_node(
                LabelSet::single(doc_label.clone()),
                props(&embedding, Value::Vector(vector(&[0.0, 0.0]))),
            )
            .unwrap();
        let non_vector = mutator
            .create_node(
                LabelSet::single(doc_label),
                props(&other, Value::String(db_string("skip").unwrap())),
            )
            .unwrap();
        mutator
            .create_edge(link.clone(), anchor, out_far, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(link.clone(), anchor, out_near, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(link.clone(), anchor, out_near, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(link.clone(), anchor, deleted, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(link.clone(), anchor, non_vector, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(other_link.clone(), anchor, incoming, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(link.clone(), incoming, anchor, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        (anchor, out_near, out_far, incoming, deleted, non_vector)
    };
    {
        let mut txn = shared.begin_write();
        txn.mutator().delete_node(deleted).unwrap();
        txn.commit().unwrap();
    }

    let outgoing = shared
        .score_vector_neighbors_checked(
            &embedding,
            &vector(&[2.2, 0.0]),
            anchor,
            VectorNeighborSearchOptions::new(
                &link,
                VectorNeighborDirection::Outgoing,
                VectorMetric::SquaredEuclidean,
                10,
            ),
            CancellationChecker::disabled(),
        )
        .unwrap();
    assert_eq!(
        outgoing.iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![out_near, out_far]
    );

    let incoming_hits = shared
        .score_vector_neighbors_checked(
            &embedding,
            &vector(&[2.2, 0.0]),
            anchor,
            VectorNeighborSearchOptions::new(
                &link,
                VectorNeighborDirection::Incoming,
                VectorMetric::SquaredEuclidean,
                10,
            ),
            CancellationChecker::disabled(),
        )
        .unwrap();
    assert_eq!(
        incoming_hits
            .iter()
            .map(|hit| hit.node_id)
            .collect::<Vec<_>>(),
        vec![incoming]
    );

    let both = shared
        .score_vector_neighbors_checked(
            &embedding,
            &vector(&[2.2, 0.0]),
            anchor,
            VectorNeighborSearchOptions::new(
                &link,
                VectorNeighborDirection::Both,
                VectorMetric::SquaredEuclidean,
                10,
            ),
            CancellationChecker::disabled(),
        )
        .unwrap();
    assert_eq!(
        both.iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![out_near, incoming, out_far]
    );

    let missing_anchor = shared
        .score_vector_neighbors_checked(
            &embedding,
            &vector(&[2.2, 0.0]),
            non_vector,
            VectorNeighborSearchOptions::new(
                &other_link,
                VectorNeighborDirection::Outgoing,
                VectorMetric::SquaredEuclidean,
                10,
            ),
            CancellationChecker::disabled(),
        )
        .unwrap();
    assert!(missing_anchor.is_empty());
}

#[test]
fn expand_vector_candidate_set_preserves_roots_and_walks_labeled_edges() {
    let shared = SharedGraph::new(GraphId::new(982));
    let root_label = db_string("vector.expand.root").unwrap();
    let doc_label = db_string("vector.expand.doc").unwrap();
    let support = db_string("SUPPORTS").unwrap();
    let other = db_string("MENTIONS").unwrap();
    let (root_a, root_b, out_a, out_b, incoming, wrong_label) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let root_a = mutator
            .create_node(LabelSet::single(root_label.clone()), PropertyMap::new())
            .unwrap();
        let root_b = mutator
            .create_node(LabelSet::single(root_label), PropertyMap::new())
            .unwrap();
        let out_a = mutator
            .create_node(LabelSet::single(doc_label.clone()), PropertyMap::new())
            .unwrap();
        let out_b = mutator
            .create_node(LabelSet::single(doc_label.clone()), PropertyMap::new())
            .unwrap();
        let incoming = mutator
            .create_node(LabelSet::single(doc_label.clone()), PropertyMap::new())
            .unwrap();
        let wrong_label = mutator
            .create_node(LabelSet::single(doc_label), PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(support.clone(), root_a, out_b, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(support.clone(), root_a, out_b, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(support.clone(), root_b, out_a, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(support.clone(), incoming, root_a, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(other.clone(), root_a, wrong_label, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        (root_a, root_b, out_a, out_b, incoming, wrong_label)
    };
    let roots = VectorCandidateSet::from_nodes([root_b, root_a, root_a]);

    let outgoing =
        shared.expand_vector_candidate_set(&roots, &support, VectorNeighborDirection::Outgoing);
    assert_eq!(outgoing.as_nodes(), &[root_a, root_b, out_a, out_b]);

    let incoming_set =
        shared.expand_vector_candidate_set(&roots, &support, VectorNeighborDirection::Incoming);
    assert_eq!(incoming_set.as_nodes(), &[root_a, root_b, incoming]);

    let both = shared.expand_vector_candidate_set(&roots, &support, VectorNeighborDirection::Both);
    assert_eq!(both.as_nodes(), &[root_a, root_b, out_a, out_b, incoming]);

    let missing =
        shared.expand_vector_candidate_set(&roots, &other, VectorNeighborDirection::Incoming);
    assert_eq!(missing.as_nodes(), &[root_a, root_b]);
    assert!(!both.as_nodes().contains(&wrong_label));
}

#[test]
fn expand_vector_candidate_set_handles_empty_and_cancelled_inputs() {
    let shared = SharedGraph::new(GraphId::new(983));
    let link = db_string("EXPANDS_TO").unwrap();
    let empty = VectorCandidateSet::default();
    assert!(
        shared
            .expand_vector_candidate_set(&empty, &link, VectorNeighborDirection::Both)
            .is_empty()
    );

    let token = CancellationToken::new();
    token.cancel();
    let err = shared
        .expand_vector_candidate_set_checked(
            &empty,
            &link,
            VectorNeighborDirection::Both,
            CancellationChecker::new(Some(&token), None),
        )
        .expect_err("cancelled expansion must report cancellation");
    assert!(matches!(err, VectorSearchError::Cancelled));
}

#[test]
fn score_vector_neighbors_batch_matches_single_anchor_queries() {
    let shared = SharedGraph::new(GraphId::new(981));
    let anchor_label = db_string("vector.neighbor.batch.anchor").unwrap();
    let doc_label = db_string("vector.neighbor.batch.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    let link = db_string("NEAR").unwrap();
    let (anchor_a, anchor_b, ids) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let anchor_a = mutator
            .create_node(LabelSet::single(anchor_label.clone()), PropertyMap::new())
            .unwrap();
        let anchor_b = mutator
            .create_node(LabelSet::single(anchor_label), PropertyMap::new())
            .unwrap();
        let mut ids = Vec::new();
        for value in 0..8 {
            ids.push(
                mutator
                    .create_node(
                        LabelSet::single(doc_label.clone()),
                        props(&embedding, Value::Vector(vector(&[value as f32, 0.0]))),
                    )
                    .unwrap(),
            );
        }
        for &node in &[ids[1], ids[2], ids[5]] {
            mutator
                .create_edge(link.clone(), anchor_a, node, PropertyMap::new())
                .unwrap();
        }
        for &node in &[ids[4], ids[6], ids[7]] {
            mutator
                .create_edge(link.clone(), anchor_b, node, PropertyMap::new())
                .unwrap();
        }
        txn.commit().unwrap();
        (anchor_a, anchor_b, ids)
    };
    let queries = vec![vector(&[2.1, 0.0]), vector(&[6.2, 0.0])];
    let anchors = vec![anchor_a, anchor_b];

    let batched = shared
        .score_vector_neighbors_batch_checked(
            &embedding,
            &queries,
            &anchors,
            VectorNeighborSearchOptions::new(
                &link,
                VectorNeighborDirection::Outgoing,
                VectorMetric::SquaredEuclidean,
                2,
            ),
            CancellationChecker::disabled(),
        )
        .unwrap();
    let singles: Vec<_> = queries
        .iter()
        .zip(&anchors)
        .map(|(query, anchor)| {
            shared
                .score_vector_neighbors_checked(
                    &embedding,
                    query,
                    *anchor,
                    VectorNeighborSearchOptions::new(
                        &link,
                        VectorNeighborDirection::Outgoing,
                        VectorMetric::SquaredEuclidean,
                        2,
                    ),
                    CancellationChecker::disabled(),
                )
                .unwrap()
        })
        .collect();

    assert_eq!(batched, singles);
    assert_eq!(
        batched[0].iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![ids[2], ids[1]]
    );
    assert_eq!(
        batched[1].iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![ids[6], ids[7]]
    );

    let err = shared
        .score_vector_neighbors_batch_checked(
            &embedding,
            &queries,
            &[anchor_a],
            VectorNeighborSearchOptions::new(
                &link,
                VectorNeighborDirection::Outgoing,
                VectorMetric::SquaredEuclidean,
                2,
            ),
            CancellationChecker::disabled(),
        )
        .expect_err("query/anchor count mismatch must error");
    assert!(matches!(
        err,
        VectorSearchError::BatchLengthMismatch {
            queries: 2,
            candidate_sets: 1
        }
    ));
}
