//! Integration tests for `dijkstra` per spec 16 §E13–§E16.

use selene_algorithms::{
    GraphProjection, PathResult, PathfindingError, ProjectionConfig, dijkstra,
};
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_graph::SharedGraph;

fn istr(name: &str) -> IStr {
    intern(name).unwrap()
}

fn weight_props(value: f64) -> PropertyMap {
    PropertyMap::from_pairs([(istr("w"), Value::Float(value))]).unwrap()
}

fn build_proj_weighted(shared: &SharedGraph) -> GraphProjection {
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

fn build_proj_unweighted(shared: &SharedGraph) -> GraphProjection {
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

/// Build a graph with `count` nodes and the supplied weighted directed edges.
fn build_weighted_graph(count: usize, edges: &[(usize, usize, f64)]) -> (SharedGraph, Vec<NodeId>) {
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

fn build_unweighted_graph(count: usize, edges: &[(usize, usize)]) -> (SharedGraph, Vec<NodeId>) {
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
    for &(s, t) in edges {
        txn.mutator()
            .create_edge(rel, nodes[s], nodes[t], PropertyMap::new())
            .unwrap();
    }
    txn.commit().unwrap();
    (shared, nodes)
}

#[test]
fn dijkstra_empty_projection_returns_none() {
    let shared = SharedGraph::new(GraphId::new(1));
    let proj = build_proj_unweighted(&shared);
    // Empty projection — no nodes, so contains() is false for any NodeId.
    let result = dijkstra(&proj, NodeId::new(1), NodeId::new(2)).unwrap();
    assert!(result.is_none());
}

#[test]
fn dijkstra_from_not_in_projection_returns_none() {
    let (shared, nodes) = build_unweighted_graph(2, &[(0, 1)]);
    let proj = build_proj_unweighted(&shared);
    // NodeId(999) is not in the projection.
    assert!(
        dijkstra(&proj, NodeId::new(999), nodes[1])
            .unwrap()
            .is_none()
    );
}

#[test]
fn dijkstra_from_equals_to_singleton_path() {
    let (shared, nodes) = build_unweighted_graph(1, &[]);
    let proj = build_proj_unweighted(&shared);
    let result = dijkstra(&proj, nodes[0], nodes[0]).unwrap().unwrap();
    assert_eq!(
        result,
        PathResult {
            nodes: vec![nodes[0]],
            cost: 0.0,
        }
    );
}

#[test]
fn dijkstra_two_node_single_edge() {
    let (shared, nodes) = build_weighted_graph(2, &[(0, 1, 5.5)]);
    let proj = build_proj_weighted(&shared);
    let result = dijkstra(&proj, nodes[0], nodes[1]).unwrap().unwrap();
    assert_eq!(result.nodes, vec![nodes[0], nodes[1]]);
    assert_eq!(result.cost, 5.5);
}

#[test]
fn dijkstra_chooses_shorter_of_two_routes() {
    // n0 --10--> n1 --5--> n2 (cost 15); n0 --20--> n2 (cost 20). Pick 15.
    let (shared, nodes) = build_weighted_graph(3, &[(0, 1, 10.0), (1, 2, 5.0), (0, 2, 20.0)]);
    let proj = build_proj_weighted(&shared);
    let result = dijkstra(&proj, nodes[0], nodes[2]).unwrap().unwrap();
    assert_eq!(result.nodes, vec![nodes[0], nodes[1], nodes[2]]);
    assert_eq!(result.cost, 15.0);
}

#[test]
fn dijkstra_unreachable_returns_none() {
    // n0 -> n1; n2 isolated.
    let (shared, nodes) = build_unweighted_graph(3, &[(0, 1)]);
    let proj = build_proj_unweighted(&shared);
    assert!(dijkstra(&proj, nodes[0], nodes[2]).unwrap().is_none());
}

#[test]
fn dijkstra_directed_edge_not_traversable_in_reverse() {
    // n0 -> n1 only; cannot go n1 -> n0.
    let (shared, nodes) = build_unweighted_graph(2, &[(0, 1)]);
    let proj = build_proj_unweighted(&shared);
    assert!(dijkstra(&proj, nodes[1], nodes[0]).unwrap().is_none());
}

#[test]
fn dijkstra_unweighted_uses_unit_weights() {
    // Each edge weight defaults to 1.0; 3-hop path has cost 3.0.
    let (shared, nodes) = build_unweighted_graph(4, &[(0, 1), (1, 2), (2, 3)]);
    let proj = build_proj_unweighted(&shared);
    let result = dijkstra(&proj, nodes[0], nodes[3]).unwrap().unwrap();
    assert_eq!(result.nodes, vec![nodes[0], nodes[1], nodes[2], nodes[3]]);
    assert_eq!(result.cost, 3.0);
}

#[test]
fn dijkstra_negative_weight_raises_error() {
    // §E15 — negative finite weights raise NegativeWeight on traversal.
    let (shared, nodes) = build_weighted_graph(2, &[(0, 1, -1.0)]);
    let proj = build_proj_weighted(&shared);
    let err = dijkstra(&proj, nodes[0], nodes[1]).unwrap_err();
    let PathfindingError::NegativeWeight {
        source_node,
        target_node,
        weight,
    } = err
    else {
        panic!("expected NegativeWeight, got {err:?}");
    };
    assert_eq!(source_node, nodes[0]);
    assert_eq!(target_node, nodes[1]);
    assert_eq!(weight, -1.0);
}

#[test]
fn dijkstra_nan_weight_detected_before_heap_insertion() {
    // §E15 + §O.A.1 — NaN weights raise NaNWeight BEFORE the entry enters
    // the heap (else f64::total_cmp would see NaN in heap ordering).
    let (shared, nodes) = build_weighted_graph(2, &[(0, 1, f64::NAN)]);
    let proj = build_proj_weighted(&shared);
    let err = dijkstra(&proj, nodes[0], nodes[1]).unwrap_err();
    let PathfindingError::NaNWeight {
        source_node,
        target_node,
    } = err
    else {
        panic!("expected NaNWeight, got {err:?}");
    };
    assert_eq!(source_node, nodes[0]);
    assert_eq!(target_node, nodes[1]);
}

#[test]
fn dijkstra_negative_weight_at_unreachable_corner_does_not_error() {
    // §E15 — detection is lazy. Negative edge n2 -> n3 is unreachable from n0;
    // dijkstra(n0, n1) must NOT see it.
    let (shared, nodes) = build_weighted_graph(4, &[(0, 1, 1.0), (2, 3, -10.0)]);
    let proj = build_proj_weighted(&shared);
    let result = dijkstra(&proj, nodes[0], nodes[1]).unwrap();
    assert!(
        result.is_some(),
        "lazy detection: untraversed negative edges are fine"
    );
}

#[test]
fn dijkstra_equal_cost_paths_tie_break_by_smallest_predecessor_nodeid() {
    // §E16 + §O.G.2 — Two paths to n3 of EQUAL cost via different intermediates:
    //   n0 --1--> n1 --2--> n3 (via smaller-NodeId predecessor n1; cost 3)
    //   n0 --2--> n2 --1--> n3 (via larger-NodeId predecessor n2; cost 3)
    // Tie-break MUST pick the chain through n1 (smaller predecessor NodeId
    // wins per the strict-less-than rewrite rule + heap secondary tie-break).
    let (shared, nodes) =
        build_weighted_graph(4, &[(0, 1, 1.0), (1, 3, 2.0), (0, 2, 2.0), (2, 3, 1.0)]);
    let proj = build_proj_weighted(&shared);
    let result = dijkstra(&proj, nodes[0], nodes[3]).unwrap().unwrap();
    assert_eq!(result.cost, 3.0);
    assert_eq!(
        result.nodes,
        vec![nodes[0], nodes[1], nodes[3]],
        "tie-break must choose path via n1 (smallest predecessor NodeId)"
    );
}

#[test]
fn dijkstra_tie_break_smaller_predecessor_popped_after_larger() {
    // §E16 + PR #59 Codex P1 counter-example: the smaller-NodeId predecessor
    // is popped AFTER the larger-NodeId predecessor; without the equal-cost
    // rewrite rule, prev[target] would lock in the larger-NodeId predecessor.
    //
    // Layout: n0 has two outgoing edges to intermediates (n1 and n2);
    // intermediates each have one edge to target (n3). The total cost to n3
    // is 5 via BOTH paths, but the in-heap order pops n2 BEFORE n1 because
    // n0 → n2 is cheaper than n0 → n1:
    //   n0 --4--> n1 --1--> n3 (total 5; predecessor n1; n1 is the §E16 winner)
    //   n0 --1--> n2 --4--> n3 (total 5; predecessor n2; popped first)
    //
    // Without the fix, prev[n3] = n2 (locked in by strict-less-than rewrite).
    // With the fix, the equal-cost branch sees `1 (n1) < 2 (n2)` in dense
    // index space and rewrites prev[n3] = n1.
    let (shared, nodes) =
        build_weighted_graph(4, &[(0, 1, 4.0), (1, 3, 1.0), (0, 2, 1.0), (2, 3, 4.0)]);
    let proj = build_proj_weighted(&shared);
    let result = dijkstra(&proj, nodes[0], nodes[3]).unwrap().unwrap();
    assert_eq!(result.cost, 5.0);
    assert_eq!(
        result.nodes,
        vec![nodes[0], nodes[1], nodes[3]],
        "tie-break must pick path via n1 (smaller predecessor NodeId), even \
         though n2 reached n3 first via the larger-predecessor route"
    );
}

#[test]
fn dijkstra_infinity_weight_accepted_no_path_emitted() {
    // §E15 — +INFINITY weight is accepted (no error). But the strict
    // less-than rewrite rule (§E16) means `0.0 + INF` is NOT less than
    // `dist[next] = INF`, so the edge cannot improve the unreached
    // distance. The algorithm returns Ok(None): no error, no path either.
    let (shared, nodes) = build_weighted_graph(2, &[(0, 1, f64::INFINITY)]);
    let proj = build_proj_weighted(&shared);
    let result = dijkstra(&proj, nodes[0], nodes[1]).unwrap();
    assert!(
        result.is_none(),
        "INF-weight edge accepted but cannot relax; no path emitted"
    );
}
