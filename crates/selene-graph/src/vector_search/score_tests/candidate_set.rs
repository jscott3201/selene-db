use super::{props, vector};
use crate::{
    SharedGraph, VectorCandidateSet, VectorNeighborDirection, VectorNeighborSearchOptions,
    VectorNodeSearchHit,
};
use selene_core::{
    CancellationChecker, GraphId, LabelSet, NodeId, PropertyMap, Value, VectorMetric, intern,
};

#[test]
fn vector_candidate_set_sorts_and_deduplicates_nodes() {
    let set = VectorCandidateSet::from_nodes([
        NodeId::new(9),
        NodeId::new(3),
        NodeId::new(9),
        NodeId::new(1),
    ]);

    assert_eq!(
        set.as_nodes(),
        &[NodeId::new(1), NodeId::new(3), NodeId::new(9)]
    );
    assert_eq!(set.len(), 3);
    assert!(!set.is_empty());
    assert_eq!(
        set.into_nodes(),
        vec![NodeId::new(1), NodeId::new(3), NodeId::new(9)]
    );
    assert!(VectorCandidateSet::from_nodes([]).is_empty());
}

#[test]
fn vector_candidate_set_builds_from_search_hits() {
    let hits = [
        VectorNodeSearchHit {
            node_id: NodeId::new(8),
            distance: 0.2,
        },
        VectorNodeSearchHit {
            node_id: NodeId::new(3),
            distance: 0.1,
        },
        VectorNodeSearchHit {
            node_id: NodeId::new(8),
            distance: 0.0,
        },
    ];

    let set = VectorCandidateSet::from_search_hits(&hits);

    assert_eq!(set.as_nodes(), &[NodeId::new(3), NodeId::new(8)]);
}

#[test]
fn vector_candidate_set_algebra_preserves_canonical_order() {
    let left = VectorCandidateSet::from_nodes([
        NodeId::new(1),
        NodeId::new(3),
        NodeId::new(5),
        NodeId::new(7),
    ]);
    let right = VectorCandidateSet::from_nodes([
        NodeId::new(3),
        NodeId::new(4),
        NodeId::new(5),
        NodeId::new(9),
    ]);

    assert_eq!(
        left.union(&right).as_nodes(),
        &[
            NodeId::new(1),
            NodeId::new(3),
            NodeId::new(4),
            NodeId::new(5),
            NodeId::new(7),
            NodeId::new(9),
        ]
    );
    assert_eq!(
        left.intersection(&right).as_nodes(),
        &[NodeId::new(3), NodeId::new(5)]
    );
    assert_eq!(
        left.difference(&right).as_nodes(),
        &[NodeId::new(1), NodeId::new(7)]
    );
    assert_eq!(
        right.difference(&left).as_nodes(),
        &[NodeId::new(4), NodeId::new(9)]
    );
}

#[test]
fn vector_candidate_set_algebra_handles_empty_and_disjoint_sets() {
    let empty = VectorCandidateSet::default();
    let left = VectorCandidateSet::from_nodes([NodeId::new(1), NodeId::new(2)]);
    let right = VectorCandidateSet::from_nodes([NodeId::new(7), NodeId::new(9)]);

    assert_eq!(left.union(&empty), left);
    assert_eq!(empty.union(&right), right);
    assert!(left.intersection(&right).is_empty());
    assert!(empty.intersection(&right).is_empty());
    assert_eq!(left.difference(&right), left);
    assert!(left.difference(&left).is_empty());
}

#[test]
fn score_vector_nodes_batch_accepts_candidate_sets() {
    let shared = SharedGraph::new(GraphId::new(982));
    let label = intern("vector.score.candidate_set.doc").unwrap();
    let embedding = intern("embedding").unwrap();
    let ids = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let mut ids = Vec::new();
        for value in 0..6 {
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
    let queries = vec![vector(&[1.1, 0.0]), vector(&[4.2, 0.0])];
    let candidate_sets = vec![
        VectorCandidateSet::from_nodes([ids[3], ids[1], ids[1], ids[0]]),
        VectorCandidateSet::from_nodes([ids[5], ids[4], ids[2], ids[5]]),
    ];

    let hits = shared
        .score_vector_nodes_batch_checked(
            &embedding,
            &queries,
            &candidate_sets,
            VectorMetric::SquaredEuclidean,
            2,
            CancellationChecker::disabled(),
        )
        .unwrap();

    assert_eq!(
        hits[0].iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![ids[1], ids[0]]
    );
    assert_eq!(
        hits[1].iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![ids[4], ids[5]]
    );
}

#[test]
fn score_vector_candidate_set_matches_explicit_node_scoring() {
    let shared = SharedGraph::new(GraphId::new(985));
    let label = intern("vector.score.canonical_set.doc").unwrap();
    let embedding = intern("embedding").unwrap();
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
        txn.commit().unwrap();
        ids
    };
    let query = vector(&[2.2, 0.0]);
    let explicit_candidates = vec![ids[4], ids[2], ids[2], ids[1], ids[0]];
    let canonical_candidates = VectorCandidateSet::from_nodes(explicit_candidates.clone());

    let explicit = shared
        .score_vector_nodes_checked(
            &embedding,
            &query,
            &explicit_candidates,
            VectorMetric::SquaredEuclidean,
            3,
            CancellationChecker::disabled(),
        )
        .unwrap();
    let canonical = shared
        .score_vector_candidate_set_checked(
            &embedding,
            &query,
            &canonical_candidates,
            VectorMetric::SquaredEuclidean,
            3,
            CancellationChecker::disabled(),
        )
        .unwrap();

    assert_eq!(canonical, explicit);
    assert_eq!(
        canonical.iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![ids[2], ids[1], ids[4]]
    );
}

#[test]
fn score_vector_candidate_sets_batch_matches_generic_batch() {
    let shared = SharedGraph::new(GraphId::new(986));
    let label = intern("vector.score.canonical_batch.doc").unwrap();
    let embedding = intern("embedding").unwrap();
    let ids = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let mut ids = Vec::new();
        for value in 0..7 {
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
    let queries = vec![vector(&[0.9, 0.0]), vector(&[5.1, 0.0])];
    let explicit_sets = vec![
        vec![ids[3], ids[0], ids[1], ids[1]],
        vec![ids[6], ids[5], ids[4], ids[6]],
    ];
    let canonical_sets = explicit_sets
        .iter()
        .map(|set| VectorCandidateSet::from_nodes(set.iter().copied()))
        .collect::<Vec<_>>();

    let generic = shared
        .score_vector_nodes_batch_checked(
            &embedding,
            &queries,
            &explicit_sets,
            VectorMetric::SquaredEuclidean,
            2,
            CancellationChecker::disabled(),
        )
        .unwrap();
    let canonical = shared
        .score_vector_candidate_sets_batch_checked(
            &embedding,
            &queries,
            &canonical_sets,
            VectorMetric::SquaredEuclidean,
            2,
            CancellationChecker::disabled(),
        )
        .unwrap();

    assert_eq!(canonical, generic);
    assert_eq!(
        canonical[0]
            .iter()
            .map(|hit| hit.node_id)
            .collect::<Vec<_>>(),
        vec![ids[1], ids[0]]
    );
    assert_eq!(
        canonical[1]
            .iter()
            .map(|hit| hit.node_id)
            .collect::<Vec<_>>(),
        vec![ids[5], ids[6]]
    );
}

#[test]
fn vector_neighbor_candidates_filter_direction_and_normalize() {
    let shared = SharedGraph::new(GraphId::new(983));
    let anchor_label = intern("vector.candidate.anchor").unwrap();
    let doc_label = intern("vector.candidate.doc").unwrap();
    let link = intern("DEPENDS_ON").unwrap();
    let other_link = intern("MENTIONS").unwrap();
    let (anchor, node_a, node_b, node_c) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let anchor = mutator
            .create_node(LabelSet::single(anchor_label), PropertyMap::new())
            .unwrap();
        let node_a = mutator
            .create_node(LabelSet::single(doc_label.clone()), PropertyMap::new())
            .unwrap();
        let node_b = mutator
            .create_node(LabelSet::single(doc_label.clone()), PropertyMap::new())
            .unwrap();
        let node_c = mutator
            .create_node(LabelSet::single(doc_label), PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(link.clone(), anchor, node_b, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(link.clone(), anchor, node_a, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(link.clone(), anchor, node_a, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(link.clone(), node_a, anchor, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(other_link, anchor, node_c, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        (anchor, node_a, node_b, node_c)
    };

    let outgoing =
        shared.vector_neighbor_candidates(anchor, &link, VectorNeighborDirection::Outgoing);
    assert_eq!(outgoing.as_nodes(), &[node_a, node_b]);

    let incoming =
        shared.vector_neighbor_candidates(anchor, &link, VectorNeighborDirection::Incoming);
    assert_eq!(incoming.as_nodes(), &[node_a]);

    let both = shared.vector_neighbor_candidates(anchor, &link, VectorNeighborDirection::Both);
    assert_eq!(both.as_nodes(), &[node_a, node_b]);

    let other = shared.vector_neighbor_candidates(
        anchor,
        &intern("ABSENT").unwrap(),
        VectorNeighborDirection::Both,
    );
    assert!(other.is_empty());

    let missing =
        shared.vector_neighbor_candidates(node_c, &link, VectorNeighborDirection::Incoming);
    assert!(missing.is_empty());
}

#[test]
fn vector_neighbor_candidate_set_scores_like_neighbor_search() {
    let shared = SharedGraph::new(GraphId::new(984));
    let anchor_label = intern("vector.candidate.score.anchor").unwrap();
    let doc_label = intern("vector.candidate.score.doc").unwrap();
    let embedding = intern("embedding").unwrap();
    let link = intern("SUPPORTS").unwrap();
    let (anchor, near, far) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let anchor = mutator
            .create_node(LabelSet::single(anchor_label), PropertyMap::new())
            .unwrap();
        let near = mutator
            .create_node(
                LabelSet::single(doc_label.clone()),
                props(&embedding, Value::Vector(vector(&[1.0, 0.0]))),
            )
            .unwrap();
        let far = mutator
            .create_node(
                LabelSet::single(doc_label),
                props(&embedding, Value::Vector(vector(&[5.0, 0.0]))),
            )
            .unwrap();
        mutator
            .create_edge(link.clone(), anchor, far, PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(link.clone(), anchor, near, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        (anchor, near, far)
    };
    let query = vector(&[1.2, 0.0]);
    let candidates =
        shared.vector_neighbor_candidates(anchor, &link, VectorNeighborDirection::Outgoing);

    let explicit = shared
        .score_vector_nodes_checked(
            &embedding,
            &query,
            candidates.as_nodes(),
            VectorMetric::SquaredEuclidean,
            2,
            CancellationChecker::disabled(),
        )
        .unwrap();
    let neighbor = shared
        .score_vector_neighbors_checked(
            &embedding,
            &query,
            anchor,
            VectorNeighborSearchOptions::new(
                &link,
                VectorNeighborDirection::Outgoing,
                VectorMetric::SquaredEuclidean,
                2,
            ),
            CancellationChecker::disabled(),
        )
        .unwrap();

    assert_eq!(explicit, neighbor);
    assert_eq!(
        neighbor.iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
        vec![near, far]
    );
}
