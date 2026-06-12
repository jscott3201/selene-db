use super::{props, vector};
use crate::{SharedGraph, VectorCandidateSet};
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
