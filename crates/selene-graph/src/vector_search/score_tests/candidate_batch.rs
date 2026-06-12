use super::{props, vector};
use crate::{
    SharedGraph, VectorCandidateSet, VectorNeighborDirection, VectorNeighborSearchOptions,
};
use selene_core::{
    CancellationChecker, GraphId, LabelSet, PropertyMap, Value, VectorMetric, db_string,
};

#[test]
fn score_vector_candidate_sets_batch_matches_single_set_for_repeated_candidates() {
    let shared = SharedGraph::new(GraphId::new(987));
    let label = db_string("vector.score.repeated_batch.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    let other = db_string("other").unwrap();
    let ids = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let mut ids = Vec::new();
        for value in [0.0_f32, 1.0, 4.0] {
            ids.push(
                mutator
                    .create_node(
                        LabelSet::single(label.clone()),
                        props(&embedding, Value::Vector(vector(&[value, 0.0]))),
                    )
                    .unwrap(),
            );
        }
        ids.push(
            mutator
                .create_node(
                    LabelSet::single(label.clone()),
                    props(&other, Value::String(db_string("not-vector").unwrap())),
                )
                .unwrap(),
        );
        ids.push(
            mutator
                .create_node(LabelSet::single(label), PropertyMap::new())
                .unwrap(),
        );
        txn.commit().unwrap();
        ids
    };

    let candidates = VectorCandidateSet::from_nodes(ids.iter().copied());
    let repeated_sets = vec![
        candidates.clone(),
        VectorCandidateSet::from_nodes([ids[4], ids[2], ids[1], ids[0], ids[0], ids[3]]),
    ];
    let queries = vec![vector(&[0.2, 0.0]), vector(&[3.8, 0.0])];

    let batched = shared
        .score_vector_candidate_sets_batch_checked(
            &embedding,
            &queries,
            &repeated_sets,
            VectorMetric::SquaredEuclidean,
            2,
            CancellationChecker::disabled(),
        )
        .unwrap();
    let manual = queries
        .iter()
        .map(|query| {
            shared
                .score_vector_candidate_set_checked(
                    &embedding,
                    query,
                    &candidates,
                    VectorMetric::SquaredEuclidean,
                    2,
                    CancellationChecker::disabled(),
                )
                .unwrap()
        })
        .collect::<Vec<_>>();

    assert_eq!(batched, manual);
    assert_eq!(batched[0][0].node_id, ids[0]);
    assert_eq!(batched[1][0].node_id, ids[2]);
}

#[test]
fn score_vector_candidate_sets_batch_matches_single_sets_for_mixed_repeated_candidates() {
    let shared = SharedGraph::new(GraphId::new(988));
    let label = db_string("vector.score.mixed_repeated_batch.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    let ids = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let mut ids = Vec::new();
        for value in [0.0_f32, 1.0, 2.0, 8.0, 9.0, 10.0] {
            ids.push(
                mutator
                    .create_node(
                        LabelSet::single(label.clone()),
                        props(&embedding, Value::Vector(vector(&[value, 0.0]))),
                    )
                    .unwrap(),
            );
        }
        txn.commit().unwrap();
        ids
    };
    let low = VectorCandidateSet::from_nodes([ids[0], ids[1], ids[2]]);
    let high = VectorCandidateSet::from_nodes([ids[3], ids[4], ids[5]]);
    let candidate_sets = vec![low.clone(), high.clone(), low.clone(), high.clone()];
    let queries = vec![
        vector(&[0.1, 0.0]),
        vector(&[9.8, 0.0]),
        vector(&[1.9, 0.0]),
        vector(&[8.2, 0.0]),
    ];

    let batched = shared
        .score_vector_candidate_sets_batch_checked(
            &embedding,
            &queries,
            &candidate_sets,
            VectorMetric::SquaredEuclidean,
            2,
            CancellationChecker::disabled(),
        )
        .unwrap();
    let manual = queries
        .iter()
        .zip(candidate_sets.iter())
        .map(|(query, candidates)| {
            shared
                .score_vector_candidate_set_checked(
                    &embedding,
                    query,
                    candidates,
                    VectorMetric::SquaredEuclidean,
                    2,
                    CancellationChecker::disabled(),
                )
                .unwrap()
        })
        .collect::<Vec<_>>();

    assert_eq!(batched, manual);
    assert_eq!(batched[0][0].node_id, ids[0]);
    assert_eq!(batched[1][0].node_id, ids[5]);
    assert_eq!(batched[2][0].node_id, ids[2]);
    assert_eq!(batched[3][0].node_id, ids[3]);
}

#[test]
fn score_vector_expanded_candidate_sets_batch_matches_single_sets_for_mixed_repeated_roots() {
    let shared = SharedGraph::new(GraphId::new(989));
    let label = db_string("vector.score.mixed_repeated_roots.doc").unwrap();
    let embedding = db_string("embedding").unwrap();
    let support = db_string("SUPPORTS").unwrap();
    let ids = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let mut ids = Vec::new();
        for value in [0.0_f32, 1.0, 2.0, 8.0, 9.0, 10.0] {
            ids.push(
                mutator
                    .create_node(
                        LabelSet::single(label.clone()),
                        props(&embedding, Value::Vector(vector(&[value, 0.0]))),
                    )
                    .unwrap(),
            );
        }
        mutator
            .create_edge(support.clone(), ids[0], ids[1], PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(support.clone(), ids[0], ids[2], PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(support.clone(), ids[3], ids[4], PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(support.clone(), ids[3], ids[5], PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        ids
    };
    let low_roots = VectorCandidateSet::from_nodes([ids[0]]);
    let high_roots = VectorCandidateSet::from_nodes([ids[3], ids[3]]);
    let root_sets = vec![
        low_roots.clone(),
        high_roots.clone(),
        low_roots.clone(),
        high_roots.clone(),
    ];
    let queries = vec![
        vector(&[0.1, 0.0]),
        vector(&[9.8, 0.0]),
        vector(&[1.9, 0.0]),
        vector(&[8.2, 0.0]),
    ];
    let options = VectorNeighborSearchOptions::new(
        &support,
        VectorNeighborDirection::Outgoing,
        VectorMetric::SquaredEuclidean,
        2,
    );

    let batched = shared
        .score_vector_expanded_candidate_sets_batch_checked(
            &embedding,
            &queries,
            &root_sets,
            options,
            CancellationChecker::disabled(),
        )
        .unwrap();
    let manual_expanded = root_sets
        .iter()
        .map(|roots| {
            shared.expand_vector_candidate_set(roots, options.edge_label, options.direction)
        })
        .collect::<Vec<_>>();
    let manual = shared
        .score_vector_candidate_sets_batch_checked(
            &embedding,
            &queries,
            &manual_expanded,
            options.metric,
            options.k,
            CancellationChecker::disabled(),
        )
        .unwrap();

    assert_eq!(batched, manual);
    assert_eq!(batched[0][0].node_id, ids[0]);
    assert_eq!(batched[1][0].node_id, ids[5]);
    assert_eq!(batched[2][0].node_id, ids[2]);
    assert_eq!(batched[3][0].node_id, ids[3]);
}
