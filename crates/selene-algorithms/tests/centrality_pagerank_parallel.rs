//! Parallel PageRank checks use tolerance because f64 reductions can differ
//! across Rayon partitions.

use std::num::NonZeroUsize;

use selene_algorithms::{
    GraphProjection, PageRankConfig, PageRankOrientation, Parallelism, ProjectionConfig, pagerank,
};
use selene_core::{DbString, GraphId, LabelSet, NodeId, PropertyMap};
use selene_graph::SharedGraph;

const PAGERANK_FIXED_ITER_RELATIVE_TOLERANCE: f64 = 1e-9;
const PAGERANK_DIRECTED_PARITY_ABSOLUTE_TOLERANCE: f64 = 1e-12;

fn db_string(name: &str) -> DbString {
    selene_core::db_string(name).unwrap()
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

fn build_proj_with_edge_labels(
    shared: &SharedGraph,
    edge_labels: Vec<DbString>,
) -> GraphProjection {
    let snapshot = shared.read();
    GraphProjection::build(
        &snapshot,
        &ProjectionConfig {
            name: "test".to_string(),
            node_labels: vec![],
            edge_labels,
            weight_property: None,
        },
        None,
    )
    .unwrap()
}

fn build_graph(count: usize, edges: &[(usize, usize)]) -> SharedGraph {
    let shared = SharedGraph::new(GraphId::new(85_001));
    let label = db_string("N");
    let rel = db_string("R");
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
    shared
}

fn build_label_filtered_asymmetric_graph() -> (SharedGraph, DbString) {
    let shared = SharedGraph::new(GraphId::new(96_003));
    let label = db_string("N");
    let knows = db_string("KNOWS");
    let owns = db_string("OWNS");

    let mut txn = shared.begin_write();
    let mut nodes = Vec::with_capacity(4);
    for _ in 0..4 {
        nodes.push(
            txn.mutator()
                .create_node(LabelSet::single(label.clone()), PropertyMap::new())
                .unwrap(),
        );
    }
    txn.mutator()
        .create_edge(knows.clone(), nodes[0], nodes[1], PropertyMap::new())
        .unwrap();
    txn.mutator()
        .create_edge(owns, nodes[2], nodes[3], PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();

    (shared, knows)
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
        orientation: PageRankOrientation::Natural,
        personalization: None,
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

fn assert_outputs_abs_close(
    expected: &[(NodeId, f64)],
    observed: &[(NodeId, f64)],
    tolerance: f64,
) {
    assert_eq!(observed.len(), expected.len());
    for ((expected_node, expected_score), (observed_node, observed_score)) in
        expected.iter().zip(observed)
    {
        assert_eq!(
            observed_node, expected_node,
            "parallel mode must preserve §E21 result ordering"
        );
        let absolute = (observed_score - expected_score).abs();
        assert!(
            absolute <= tolerance,
            "score for {expected_node:?} differed: expected {expected_score}, observed {observed_score}, absolute {absolute}, tolerance {tolerance}"
        );
    }
}

fn assert_outputs_exact(expected: &[(NodeId, f64)], observed: &[(NodeId, f64)]) {
    assert_eq!(observed.len(), expected.len());
    for ((expected_node, expected_score), (observed_node, observed_score)) in
        expected.iter().zip(observed)
    {
        assert_eq!(
            observed_node, expected_node,
            "Auto must preserve sequential §E21 result ordering"
        );
        assert_eq!(
            observed_score.to_bits(),
            expected_score.to_bits(),
            "Auto PageRank is the sequential policy and should not change f64 reductions"
        );
    }
}

/// Build a projection over a label that matches no node → an empty projection.
fn empty_projection() -> GraphProjection {
    let shared = SharedGraph::new(GraphId::new(85_099));
    let snapshot = shared.read();
    GraphProjection::build(
        &snapshot,
        &ProjectionConfig {
            name: "empty".to_string(),
            node_labels: vec![db_string("Nonexistent")],
            edge_labels: vec![],
            weight_property: None,
        },
        None,
    )
    .unwrap()
}

#[test]
fn pagerank_empty_projection_returns_empty_under_all_parallelism() {
    // ALGO-15: empty projection stays a cheap early return under every policy.
    // `Auto` now uses the sequential PageRank policy, while `Threads(4)` still
    // exercises the explicit Rayon path that would otherwise build a 4-thread
    // pool over zero work.
    let proj = empty_projection();
    assert_eq!(proj.node_count(), 0);

    for parallelism in [Parallelism::Sequential, Parallelism::Auto, threads4()] {
        let scores = pagerank(&proj, config(16, 0.0, parallelism));
        assert!(
            scores.is_empty(),
            "empty projection yields empty PageRank under {parallelism:?}"
        );
    }
}

#[test]
fn parallelism_effective_threads_matches_requested_pool_size() {
    // ALGO-15: `Threads(n)` reports the exact requested pool size (the
    // `ParallelRunner` builds a dedicated n-thread pool); `Sequential` reports
    // 1; `Auto` reports a positive count from the ambient pool.
    assert_eq!(threads4().effective_threads(), 4);
    assert_eq!(Parallelism::Sequential.effective_threads(), 1);
    assert!(Parallelism::Auto.effective_threads() >= 1);
}

#[test]
fn pagerank_parallel_matches_sequential_fixed_iter() {
    let proj = fixture_projection();
    let sequential = pagerank(&proj, config(32, 0.0, Parallelism::Sequential));
    let auto = pagerank(&proj, config(32, 0.0, Parallelism::Auto));
    let threaded = pagerank(&proj, config(32, 0.0, threads4()));

    assert_outputs_exact(&sequential, &auto);
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

    assert_outputs_exact(&sequential, &auto);
    assert_outputs_close(&sequential, &threaded, expected_bound);
}

#[test]
fn pagerank_auto_uses_sequential_policy() {
    let proj = fixture_projection();
    let sequential = pagerank(&proj, config(100, 1e-6, Parallelism::Sequential));
    let auto = pagerank(&proj, config(100, 1e-6, Parallelism::Auto));
    assert_outputs_exact(&sequential, &auto);
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

#[test]
fn pagerank_seq_par_parity_on_directed_dag() {
    let shared = build_graph(4, &[(0, 1), (1, 2), (2, 3)]);
    let proj = build_proj(&shared);
    let sequential = pagerank(&proj, config(100, 0.0, Parallelism::Sequential));
    let auto = pagerank(&proj, config(100, 0.0, Parallelism::Auto));
    let threaded = pagerank(&proj, config(100, 0.0, threads4()));

    assert_outputs_abs_close(
        &sequential,
        &auto,
        PAGERANK_DIRECTED_PARITY_ABSOLUTE_TOLERANCE,
    );
    assert_outputs_abs_close(
        &sequential,
        &threaded,
        PAGERANK_DIRECTED_PARITY_ABSOLUTE_TOLERANCE,
    );
}

#[test]
fn pagerank_seq_par_parity_with_personalization() {
    let shared = build_graph(4, &[(0, 1), (1, 2), (2, 3)]);
    let proj = build_proj(&shared);
    let nodes: Vec<NodeId> = proj.iter_nodes().collect();
    let personalized = |parallelism| PageRankConfig {
        damping: 0.85,
        max_iter: 100,
        tolerance: 0.0,
        parallelism,
        orientation: PageRankOrientation::Natural,
        personalization: Some(vec![(nodes[0], 2.0), (nodes[2], 1.0)]),
    };

    let sequential = pagerank(&proj, personalized(Parallelism::Sequential));
    let auto = pagerank(&proj, personalized(Parallelism::Auto));
    let threaded = pagerank(&proj, personalized(threads4()));

    assert_outputs_abs_close(
        &sequential,
        &auto,
        PAGERANK_DIRECTED_PARITY_ABSOLUTE_TOLERANCE,
    );
    assert_outputs_abs_close(
        &sequential,
        &threaded,
        PAGERANK_DIRECTED_PARITY_ABSOLUTE_TOLERANCE,
    );
}

#[test]
fn pagerank_seq_par_parity_with_undirected_personalization() {
    let shared = build_graph(4, &[(0, 1), (2, 1), (3, 2)]);
    let proj = build_proj(&shared);
    let nodes: Vec<NodeId> = proj.iter_nodes().collect();
    let personalized = |parallelism| PageRankConfig {
        damping: 0.85,
        max_iter: 64,
        tolerance: 0.0,
        parallelism,
        orientation: PageRankOrientation::Undirected,
        personalization: Some(vec![(nodes[1], 1.0)]),
    };

    let sequential = pagerank(&proj, personalized(Parallelism::Sequential));
    let auto = pagerank(&proj, personalized(Parallelism::Auto));
    let threaded = pagerank(&proj, personalized(threads4()));

    assert_outputs_abs_close(
        &sequential,
        &auto,
        PAGERANK_DIRECTED_PARITY_ABSOLUTE_TOLERANCE,
    );
    assert_outputs_abs_close(
        &sequential,
        &threaded,
        PAGERANK_DIRECTED_PARITY_ABSOLUTE_TOLERANCE,
    );
}

#[test]
fn pagerank_seq_par_parity_on_label_filtered_asymmetric() {
    let (shared, knows) = build_label_filtered_asymmetric_graph();
    let proj = build_proj_with_edge_labels(&shared, vec![knows]);
    let sequential = pagerank(&proj, config(100, 0.0, Parallelism::Sequential));
    let auto = pagerank(&proj, config(100, 0.0, Parallelism::Auto));
    let threaded = pagerank(&proj, config(100, 0.0, threads4()));

    assert_outputs_abs_close(
        &sequential,
        &auto,
        PAGERANK_DIRECTED_PARITY_ABSOLUTE_TOLERANCE,
    );
    assert_outputs_abs_close(
        &sequential,
        &threaded,
        PAGERANK_DIRECTED_PARITY_ABSOLUTE_TOLERANCE,
    );
}
