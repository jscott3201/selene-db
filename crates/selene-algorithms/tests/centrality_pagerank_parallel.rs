//! Parallel PageRank checks use tolerance because f64 reductions can differ
//! across Rayon partitions.

use std::num::NonZeroUsize;

use selene_algorithms::{GraphProjection, PageRankConfig, Parallelism, ProjectionConfig, pagerank};
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, intern};
use selene_graph::SharedGraph;

const PAGERANK_FIXED_ITER_RELATIVE_TOLERANCE: f64 = 1e-9;

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
    let shared = SharedGraph::new(GraphId::new(85_001));
    let label = istr("N");
    let rel = istr("R");
    let mut txn = shared.begin_write();
    let mut nodes = Vec::with_capacity(count);
    for _ in 0..count {
        nodes.push(
            txn.mutator()
                .create_node(LabelSet::single(label), PropertyMap::new())
                .unwrap(),
        );
    }
    for &(source, target) in edges {
        txn.mutator()
            .create_edge(rel, nodes[source], nodes[target], PropertyMap::new())
            .unwrap();
    }
    txn.commit().unwrap();
    shared
}

fn fixture_projection() -> GraphProjection {
    let shared = build_graph(
        12,
        &[
            (0, 1),
            (0, 2),
            (1, 2),
            (1, 3),
            (2, 3),
            (2, 4),
            (3, 4),
            (4, 1),
            (5, 2),
            (5, 6),
            (6, 7),
            (7, 5),
            (8, 7),
            (9, 8),
            (10, 9),
        ],
    );
    build_proj(&shared)
}

fn threads4() -> Parallelism {
    Parallelism::Threads(NonZeroUsize::new(4).expect("non-zero thread count"))
}

fn config(max_iter: usize, tolerance: f64, parallelism: Parallelism) -> PageRankConfig {
    PageRankConfig {
        damping: 0.85,
        max_iter,
        tolerance,
        parallelism,
    }
}

fn assert_outputs_close(expected: &[(NodeId, f64)], observed: &[(NodeId, f64)], tolerance: f64) {
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
            relative <= tolerance,
            "score for {expected_node:?} differed: expected {expected_score}, observed {observed_score}, relative {relative}, tolerance {tolerance}"
        );
    }
}

#[test]
fn pagerank_parallel_matches_sequential_fixed_iter() {
    let proj = fixture_projection();
    let sequential = pagerank(&proj, config(32, 0.0, Parallelism::Sequential));
    let auto = pagerank(&proj, config(32, 0.0, Parallelism::Auto));
    let threaded = pagerank(&proj, config(32, 0.0, threads4()));

    assert_outputs_close(&sequential, &auto, PAGERANK_FIXED_ITER_RELATIVE_TOLERANCE);
    assert_outputs_close(
        &sequential,
        &threaded,
        PAGERANK_FIXED_ITER_RELATIVE_TOLERANCE,
    );
}

#[test]
fn pagerank_parallel_matches_sequential_with_convergence() {
    let proj = fixture_projection();
    let tolerance = 1e-6;
    let expected_bound = PAGERANK_FIXED_ITER_RELATIVE_TOLERANCE.max(tolerance);
    let sequential = pagerank(&proj, config(100, tolerance, Parallelism::Sequential));
    let auto = pagerank(&proj, config(100, tolerance, Parallelism::Auto));
    let threaded = pagerank(&proj, config(100, tolerance, threads4()));

    assert_outputs_close(&sequential, &auto, expected_bound);
    assert_outputs_close(&sequential, &threaded, expected_bound);
}

#[test]
fn pagerank_parallel_is_stable_under_repeat() {
    let proj = fixture_projection();
    let expected = pagerank(&proj, config(32, 0.0, threads4()));

    for _ in 0..50 {
        let observed = pagerank(&proj, config(32, 0.0, threads4()));
        assert_outputs_close(&expected, &observed, PAGERANK_FIXED_ITER_RELATIVE_TOLERANCE);
    }
}
