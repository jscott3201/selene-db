//! Integration tests for `apsp` per spec 16 §E18.

use std::num::NonZeroUsize;

use selene_algorithms::{
    ApspConfig, GraphProjection, Parallelism, PathfindingError, ProjectionConfig, apsp,
};
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_graph::SharedGraph;

fn istr(name: &str) -> IStr {
    intern(name).unwrap()
}

fn weight_props(value: f64) -> PropertyMap {
    PropertyMap::from_pairs([(istr("w"), Value::Float(value))]).unwrap()
}

fn build_proj(shared: &SharedGraph) -> GraphProjection {
    let snapshot = shared.read();
    GraphProjection::build(
        &snapshot,
        &ProjectionConfig {
            name: "test".to_string(),
            node_labels: vec![],
            edge_labels: vec![],
            weight_property: Some(istr("w")),
        },
        None,
    )
    .unwrap()
}

fn apsp_config(max_nodes: usize, parallelism: Parallelism) -> ApspConfig {
    ApspConfig {
        max_nodes,
        parallelism,
    }
}

fn sequential_apsp_config(max_nodes: usize) -> ApspConfig {
    apsp_config(max_nodes, Parallelism::Sequential)
}

fn threads4() -> Parallelism {
    Parallelism::Threads(NonZeroUsize::new(4).expect("non-zero thread count"))
}

fn build_graph(count: usize, edges: &[(usize, usize, f64)]) -> (SharedGraph, Vec<NodeId>) {
    let shared = SharedGraph::new(GraphId::new(1));
    let label = istr("N");
    let rel = istr("R");
    let mut txn = shared.begin_write();
    let mut nodes = Vec::with_capacity(count);
    for _ in 0..count {
        let id = txn
            .mutator()
            .create_node(LabelSet::single(label), PropertyMap::new())
            .unwrap();
        nodes.push(id);
    }
    for &(s, t, w) in edges {
        txn.mutator()
            .create_edge(rel, nodes[s], nodes[t], weight_props(w))
            .unwrap();
    }
    txn.commit().unwrap();
    (shared, nodes)
}

#[test]
fn apsp_empty_projection_returns_empty() {
    let shared = SharedGraph::new(GraphId::new(1));
    let proj = build_proj(&shared);
    assert!(apsp(&proj, sequential_apsp_config(100)).unwrap().is_empty());
}

#[test]
fn apsp_excludes_self_pairs() {
    // §E18 — self-pairs are excluded (asymmetric to sssp's source-inclusion).
    let (shared, nodes) = build_graph(2, &[(0, 1, 5.0)]);
    let proj = build_proj(&shared);
    let result = apsp(&proj, sequential_apsp_config(10)).unwrap();
    for &(s, t, _) in &result {
        assert_ne!(s, t, "self-pair must not appear in apsp output");
    }
    let _ = nodes;
}

#[test]
fn apsp_excludes_unreachable_pairs() {
    // n0 -> n1; n2 isolated. apsp must NOT emit (n0, n2) or (n1, n2).
    let (shared, nodes) = build_graph(3, &[(0, 1, 1.0)]);
    let proj = build_proj(&shared);
    let result = apsp(&proj, sequential_apsp_config(10)).unwrap();
    for &(s, t, _) in &result {
        assert!(
            !(s == nodes[0] && t == nodes[2]),
            "(n0, n2) is unreachable; must not appear"
        );
        assert!(
            !(s == nodes[1] && t == nodes[2]),
            "(n1, n2) is unreachable; must not appear"
        );
        assert!(
            !(s == nodes[2] && (t == nodes[0] || t == nodes[1])),
            "n2 has no outgoing; must not appear as source"
        );
    }
    assert_eq!(result.len(), 1, "only (n0, n1, 1.0) should appear");
    assert_eq!(result[0], (nodes[0], nodes[1], 1.0));
}

#[test]
fn apsp_too_large_rejected_with_caller_limit() {
    let (shared, _) = build_graph(5, &[]);
    let proj = build_proj(&shared);
    let err = apsp(&proj, sequential_apsp_config(3)).unwrap_err();
    let PathfindingError::TooLarge { nodes, limit } = err else {
        panic!("expected TooLarge, got {err:?}");
    };
    assert_eq!(nodes, 5);
    assert_eq!(limit, 3);
}

#[test]
fn apsp_triangle_directed() {
    // n0 -> n1 (1.0), n1 -> n2 (2.0), n0 -> n2 (10.0)
    // Distances: (n0, n1, 1), (n0, n2, 3), (n1, n2, 2). Sorted ASC.
    let (shared, nodes) = build_graph(3, &[(0, 1, 1.0), (1, 2, 2.0), (0, 2, 10.0)]);
    let proj = build_proj(&shared);
    let result = apsp(&proj, sequential_apsp_config(10)).unwrap();
    assert_eq!(
        result,
        vec![
            (nodes[0], nodes[1], 1.0),
            (nodes[0], nodes[2], 3.0),
            (nodes[1], nodes[2], 2.0),
        ]
    );
}

#[test]
fn apsp_result_sorted_by_source_then_target() {
    let (shared, nodes) = build_graph(4, &[(0, 1, 1.0), (1, 2, 1.0), (2, 3, 1.0), (0, 3, 5.0)]);
    let proj = build_proj(&shared);
    let result = apsp(&proj, sequential_apsp_config(10)).unwrap();
    for w in result.windows(2) {
        let lhs = (w[0].0.get(), w[0].1.get());
        let rhs = (w[1].0.get(), w[1].1.get());
        assert!(lhs < rhs, "apsp result not ASC by (source, target)");
    }
    let _ = nodes;
}

#[test]
fn apsp_negative_weight_propagates_error() {
    // The error fires on whichever source first traverses the negative edge.
    let (shared, _) = build_graph(2, &[(0, 1, -1.0)]);
    let proj = build_proj(&shared);
    let err = apsp(&proj, sequential_apsp_config(10)).unwrap_err();
    assert!(matches!(err, PathfindingError::NegativeWeight { .. }));
}

#[test]
fn apsp_nan_weight_propagates_error() {
    let (shared, _) = build_graph(2, &[(0, 1, f64::NAN)]);
    let proj = build_proj(&shared);
    let err = apsp(&proj, sequential_apsp_config(10)).unwrap_err();
    assert!(matches!(err, PathfindingError::NaNWeight { .. }));
}

#[test]
fn apsp_consistent_with_per_source_sssp() {
    // §H — APSP MUST equal { (s, t, d) | s != t and (t, d) in sssp(proj, s) }.
    use selene_algorithms::sssp;
    let (shared, _) = build_graph(
        4,
        &[
            (0, 1, 1.0),
            (1, 2, 2.0),
            (2, 3, 3.0),
            (0, 3, 100.0),
            (1, 3, 7.0),
        ],
    );
    let proj = build_proj(&shared);
    let apsp_result = apsp(&proj, sequential_apsp_config(10)).unwrap();
    let mut expected: Vec<(NodeId, NodeId, f64)> = Vec::new();
    for s in proj.iter_nodes() {
        for (t, d) in sssp(&proj, s).unwrap() {
            if s != t {
                expected.push((s, t, d));
            }
        }
    }
    expected.sort_by_key(|&(s, t, _)| (s.get(), t.get()));
    assert_eq!(apsp_result, expected);
}

#[test]
fn apsp_parallel_modes_match_sequential_output() {
    let (shared, _) = build_graph(
        6,
        &[
            (0, 1, 1.0),
            (1, 2, 2.0),
            (2, 3, 3.0),
            (3, 4, 4.0),
            (4, 5, 5.0),
            (0, 5, 100.0),
            (1, 4, 1.5),
        ],
    );
    let proj = build_proj(&shared);
    let sequential = apsp(&proj, sequential_apsp_config(10)).unwrap();
    let auto = apsp(&proj, apsp_config(10, Parallelism::Auto)).unwrap();
    let threaded = apsp(&proj, apsp_config(10, threads4())).unwrap();

    assert_eq!(auto, sequential);
    assert_eq!(threaded, sequential);
}

#[test]
fn apsp_parallel_result_is_deterministic_under_repetition() {
    let (shared, _) = build_graph(
        8,
        &[
            (0, 1, 1.0),
            (0, 2, 1.0),
            (1, 3, 1.0),
            (2, 3, 1.0),
            (3, 4, 2.0),
            (4, 5, 2.0),
            (5, 6, 2.0),
            (6, 7, 2.0),
            (1, 7, 20.0),
        ],
    );
    let proj = build_proj(&shared);
    let expected = apsp(&proj, apsp_config(10, threads4())).unwrap();

    for _ in 0..50 {
        let observed = apsp(&proj, apsp_config(10, threads4())).unwrap();
        assert_eq!(observed, expected);
    }
}

#[test]
fn apsp_parallel_propagates_pathfinding_errors() {
    let (shared, _) = build_graph(2, &[(0, 1, -1.0)]);
    let proj = build_proj(&shared);
    let err = apsp(&proj, apsp_config(10, threads4())).unwrap_err();

    assert!(matches!(err, PathfindingError::NegativeWeight { .. }));
}
