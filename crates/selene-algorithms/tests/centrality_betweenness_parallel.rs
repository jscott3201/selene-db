//! Parallel betweenness checks use relative tolerance because f64 reduction
//! order differs across Rayon partitions.

use std::num::NonZeroUsize;

use selene_algorithms::{
    BetweennessConfig, GraphProjection, Parallelism, ProjectionConfig, betweenness,
};
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, intern};
use selene_graph::SharedGraph;

const PARALLEL_BETWEENNESS_RELATIVE_TOLERANCE: f64 = 1e-9;

fn istr(name: &str) -> IStr {
    intern(name).unwrap()
}

fn build_proj(shared: &SharedGraph) -> GraphProjection {
    let snapshot = shared.read();
    GraphProjection::build(
        &snapshot,
        &ProjectionConfig {
            name: "test".to_string(),
            node_labels: vec![],
            edge_labels: vec![],
            weight_property: None,
        },
        None,
    )
    .unwrap()
}

fn build_graph(count: usize, edges: &[(usize, usize)]) -> SharedGraph {
    build_graph_with_nodes(count, edges).0
}

fn build_graph_with_nodes(count: usize, edges: &[(usize, usize)]) -> (SharedGraph, Vec<NodeId>) {
    let shared = SharedGraph::new(GraphId::new(84_001));
    let label = istr("N");
    let rel = istr("R");
    let mut txn = shared.begin_write();
    let mut nodes = Vec::with_capacity(count);
    for _ in 0..count {
        nodes.push(
            txn.mutator()
                .create_node(LabelSet::single(label.clone()), PropertyMap::new())
                .unwrap(),
        );
    }
    for &(source, target) in edges {
        txn.mutator()
            .create_edge(
                rel.clone(),
                nodes[source],
                nodes[target],
                PropertyMap::new(),
            )
            .unwrap();
    }
    txn.commit().unwrap();
    (shared, nodes)
}

fn betweenness_config(sample_size: Option<usize>, parallelism: Parallelism) -> BetweennessConfig {
    BetweennessConfig {
        sample_size,
        parallelism,
    }
}

fn threads4() -> Parallelism {
    Parallelism::Threads(NonZeroUsize::new(4).expect("non-zero thread count"))
}

fn fixture_projection() -> GraphProjection {
    let shared = build_graph(
        10,
        &[
            (0, 1),
            (0, 2),
            (1, 3),
            (2, 3),
            (3, 4),
            (4, 5),
            (1, 6),
            (6, 5),
            (2, 7),
            (7, 5),
            (5, 8),
            (8, 9),
            (3, 9),
        ],
    );
    build_proj(&shared)
}

fn assert_outputs_close(expected: &[(NodeId, f64)], observed: &[(NodeId, f64)]) {
    assert_eq!(observed.len(), expected.len());
    for ((expected_node, expected_score), (observed_node, observed_score)) in
        expected.iter().zip(observed)
    {
        assert_eq!(
            observed_node, expected_node,
            "parallel mode must preserve §E21 result ordering"
        );
        let relative = (observed_score - expected_score).abs() / expected_score.abs().max(1.0);
        assert!(
            relative <= PARALLEL_BETWEENNESS_RELATIVE_TOLERANCE,
            "score for {expected_node:?} differed: expected {expected_score}, observed {observed_score}, relative {relative}"
        );
    }
}

#[test]
fn betweenness_parallel_matches_sequential_within_epsilon() {
    let proj = fixture_projection();
    let sequential = betweenness(&proj, betweenness_config(None, Parallelism::Sequential));
    let auto = betweenness(&proj, betweenness_config(None, Parallelism::Auto));
    let threaded = betweenness(&proj, betweenness_config(None, threads4()));

    assert_outputs_close(&sequential, &auto);
    assert_outputs_close(&sequential, &threaded);
}

#[test]
fn betweenness_parallel_is_stable_under_repeat() {
    let proj = fixture_projection();
    let expected = betweenness(&proj, betweenness_config(None, threads4()));

    for _ in 0..50 {
        let observed = betweenness(&proj, betweenness_config(None, threads4()));
        assert_outputs_close(&expected, &observed);
    }
}

#[test]
fn betweenness_parallel_with_sample_size_matches_sequential() {
    let proj = fixture_projection();
    let sequential = betweenness(&proj, betweenness_config(Some(4), Parallelism::Sequential));
    let threaded = betweenness(&proj, betweenness_config(Some(4), threads4()));

    assert_outputs_close(&sequential, &threaded);
}

#[test]
fn betweenness_parallel_directed_triangle_has_unnormalized_unit_scores() {
    let (shared, nodes) = build_graph_with_nodes(3, &[(0, 1), (1, 2), (2, 0)]);
    let proj = build_proj(&shared);
    let result = betweenness(&proj, betweenness_config(None, threads4()));

    assert_eq!(
        result.iter().map(|&(node, _)| node).collect::<Vec<_>>(),
        nodes
    );
    for &(node, score) in &result {
        assert_eq!(
            score, 1.0,
            "parallel directed 3-cycle score for {node:?} must stay unnormalized"
        );
    }
}
