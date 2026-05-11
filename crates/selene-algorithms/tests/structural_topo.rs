//! Integration tests for topological_sort under the projection's directed
//! view (spec 16 §E09, §E10, §E12; BRIEF-52).

use selene_algorithms::{GraphProjection, ProjectionConfig, TopoSortError, topological_sort};
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, intern};
use selene_graph::SharedGraph;

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

fn build_graph(count: usize, edges: &[(usize, usize)]) -> (SharedGraph, Vec<NodeId>) {
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
fn topo_empty_projection_is_ok_empty() {
    let shared = SharedGraph::new(GraphId::new(1));
    let proj = build_proj(&shared);
    let result = topological_sort(&proj).expect("empty graph topo sort is Ok per §E09");
    assert!(result.is_empty());
}

#[test]
fn topo_single_node() {
    let (shared, nodes) = build_graph(1, &[]);
    let proj = build_proj(&shared);
    let result = topological_sort(&proj).unwrap();
    assert_eq!(result, vec![(nodes[0], 0)]);
}

#[test]
fn topo_linear_chain() {
    // n1 -> n2 -> n3 — only valid topo order is [n1, n2, n3].
    let (shared, nodes) = build_graph(3, &[(0, 1), (1, 2)]);
    let proj = build_proj(&shared);
    let result = topological_sort(&proj).unwrap();
    assert_eq!(result, vec![(nodes[0], 0), (nodes[1], 1), (nodes[2], 2)]);
}

#[test]
fn topo_diamond_dag() {
    // n1 -> n2, n1 -> n3, n2 -> n4, n3 -> n4. Both n2 and n3 are zero-in-degree
    // after n1; per §E12 they MUST be tie-broken ASC by NodeId.
    let (shared, nodes) = build_graph(4, &[(0, 1), (0, 2), (1, 3), (2, 3)]);
    let proj = build_proj(&shared);
    let result = topological_sort(&proj).unwrap();
    assert_eq!(
        result,
        vec![(nodes[0], 0), (nodes[1], 1), (nodes[2], 2), (nodes[3], 3),]
    );
}

#[test]
fn topo_multiple_zero_indegree_roots_sorted_asc() {
    // Three disconnected isolated nodes — all zero-in-degree.
    let (shared, nodes) = build_graph(3, &[]);
    let proj = build_proj(&shared);
    let result = topological_sort(&proj).unwrap();
    assert_eq!(result, vec![(nodes[0], 0), (nodes[1], 1), (nodes[2], 2)]);
}

#[test]
fn topo_cycle_returns_not_a_dag() {
    // n1 -> n2 -> n3 -> n1 — Kahn's stalls; cycle_hint should name a node
    // that retains positive in-degree.
    let (shared, nodes) = build_graph(3, &[(0, 1), (1, 2), (2, 0)]);
    let proj = build_proj(&shared);
    let err = topological_sort(&proj).unwrap_err();
    let TopoSortError::NotADag { cycle_hint } = err else {
        panic!("expected NotADag, got {err:?}");
    };
    let hint = cycle_hint.expect("cycle hint should be present for a non-empty cycle");
    // The hint must be one of the cycle's nodes.
    assert!(nodes.contains(&hint), "cycle hint {hint:?} not in cycle");
}

#[test]
fn topo_partial_cycle_returns_not_a_dag() {
    // DAG portion: n1 -> n2; cycle portion: n3 -> n4 -> n3. The whole graph is
    // not a DAG — Kahn's reports an error and the hint points at a cycle node.
    let (shared, nodes) = build_graph(4, &[(0, 1), (2, 3), (3, 2)]);
    let proj = build_proj(&shared);
    let err = topological_sort(&proj).unwrap_err();
    let TopoSortError::NotADag { cycle_hint } = err else {
        panic!("expected NotADag, got {err:?}");
    };
    let hint = cycle_hint.expect("cycle hint should be present");
    assert!(
        hint == nodes[2] || hint == nodes[3],
        "hint must point at a cycle node (n3 or n4), got {hint:?}"
    );
}

#[test]
fn topo_self_loop_is_cycle() {
    // A self-loop n1 -> n1 is a 1-cycle; Kahn's MUST stall.
    let (shared, nodes) = build_graph(1, &[(0, 0)]);
    let proj = build_proj(&shared);
    let err = topological_sort(&proj).unwrap_err();
    let TopoSortError::NotADag { cycle_hint } = err else {
        panic!("expected NotADag, got {err:?}");
    };
    assert_eq!(cycle_hint, Some(nodes[0]));
}

#[test]
fn topo_positions_are_dense_zero_based() {
    // Positions must form 0..N with no gaps regardless of NodeId values.
    let (shared, nodes) = build_graph(5, &[(0, 1), (1, 2), (2, 3), (3, 4)]);
    let proj = build_proj(&shared);
    let result = topological_sort(&proj).unwrap();
    let positions: Vec<usize> = result.iter().map(|&(_, p)| p).collect();
    assert_eq!(positions, vec![0, 1, 2, 3, 4]);
    // And the node order matches the chain.
    let ordered: Vec<NodeId> = result.iter().map(|&(n, _)| n).collect();
    assert_eq!(ordered, nodes);
}
