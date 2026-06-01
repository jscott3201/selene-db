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
            .create_node(LabelSet::single(label.clone()), PropertyMap::new())
            .unwrap();
        nodes.push(id);
    }
    for &(s, t) in edges {
        txn.mutator()
            .create_edge(rel.clone(), nodes[s], nodes[t], PropertyMap::new())
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
    // not a DAG — Kahn's reports an error and the hint MUST name a node from
    // the cyclic subset, never one of the DAG-portion nodes.
    //
    // ALGO-16: a weaker test would only assert the error *variant*
    // (`matches!(NotADag { .. })`). Here we pin the hint to the exact cyclic
    // subset {n3, n4} and assert it is NOT one of the DAG nodes {n1, n2}: n1
    // (in-degree 0) and n2 (in-degree 1, satisfied by n1) both drain during the
    // sweep, so only the cycle members retain positive in-degree when Kahn's
    // stalls. Tolerant of *which* cycle member surfaces.
    let (shared, nodes) = build_graph(4, &[(0, 1), (2, 3), (3, 2)]);
    let proj = build_proj(&shared);
    let err = topological_sort(&proj).unwrap_err();
    let TopoSortError::NotADag { cycle_hint } = err else {
        panic!("expected NotADag, got {err:?}");
    };
    let hint = cycle_hint.expect("cycle hint should be present");

    let cyclic_subset = [nodes[2], nodes[3]];
    let dag_subset = [nodes[0], nodes[1]];
    assert!(
        cyclic_subset.contains(&hint),
        "hint {hint:?} must be a cycle member (one of {cyclic_subset:?})"
    );
    assert!(
        !dag_subset.contains(&hint),
        "hint {hint:?} must NOT be a DAG-portion node ({dag_subset:?}) — those drain before the stall"
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
fn topo_per_batch_tie_break_is_level_batched_not_global_ordered() {
    // Documents the §E12 per-batch sort behavior: with edges `n1 -> n2` and
    // isolated `n3`, both n1 and n3 are zero-in-degree at the start. The
    // per-batch sort emits the entire current batch [n1, n3] in ASC order
    // BEFORE checking newly-zero nodes (n2 becomes ready mid-batch); n2 is
    // then drained in the next batch.
    //
    // A "global ordered frontier" formulation would produce [n1, n2, n3]
    // instead. Both are valid topological orders; per-batch matches donor
    // `aether-db-algorithms/src/structural.rs:266-329`. This test asserts
    // the chosen per-batch behavior so future reviewers see the intent.
    let (shared, nodes) = build_graph(3, &[(0, 1)]);
    let proj = build_proj(&shared);
    let result = topological_sort(&proj).unwrap();
    assert_eq!(
        result,
        vec![(nodes[0], 0), (nodes[2], 1), (nodes[1], 2)],
        "per-batch sort: [n1, n3] emitted first, then [n2]"
    );
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
