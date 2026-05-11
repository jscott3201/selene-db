//! Integration tests for articulation_points and bridges under the
//! projection's undirected view (spec 16 §E09, §E10, §E11, §E12; BRIEF-52).

use proptest::prelude::*;
use selene_algorithms::{GraphProjection, ProjectionConfig, articulation_points, bridges};
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
fn articulation_empty_projection() {
    let shared = SharedGraph::new(GraphId::new(1));
    let proj = build_proj(&shared);
    assert!(articulation_points(&proj).is_empty(), "spec §E09");
    assert!(bridges(&proj).is_empty(), "spec §E09");
}

#[test]
fn articulation_single_node_no_ap_no_bridge() {
    let (shared, _) = build_graph(1, &[]);
    let proj = build_proj(&shared);
    assert!(articulation_points(&proj).is_empty());
    assert!(bridges(&proj).is_empty());
}

#[test]
fn articulation_two_nodes_one_edge_is_bridge_no_ap() {
    // n1 — n2: every edge in a 2-node tree is a bridge; neither endpoint is
    // an articulation point.
    let (shared, nodes) = build_graph(2, &[(0, 1)]);
    let proj = build_proj(&shared);
    assert!(articulation_points(&proj).is_empty());
    assert_eq!(bridges(&proj), vec![(nodes[0], nodes[1])]);
}

#[test]
fn articulation_linear_path_middle_node_is_ap_all_edges_bridges() {
    // n1 — n2 — n3. n2 is the only articulation point. Both edges are bridges.
    let (shared, nodes) = build_graph(3, &[(0, 1), (1, 2)]);
    let proj = build_proj(&shared);
    assert_eq!(articulation_points(&proj), vec![nodes[1]]);
    assert_eq!(
        bridges(&proj),
        vec![(nodes[0], nodes[1]), (nodes[1], nodes[2])]
    );
}

#[test]
fn articulation_triangle_has_no_ap_no_bridge() {
    // 3-cycle n1 - n2 - n3 - n1 (directed but undirected view is a 3-cycle).
    let (shared, _) = build_graph(3, &[(0, 1), (1, 2), (2, 0)]);
    let proj = build_proj(&shared);
    assert!(articulation_points(&proj).is_empty());
    assert!(bridges(&proj).is_empty());
}

#[test]
fn articulation_star_center_is_ap_all_spokes_bridges() {
    // Star: n1 — n2, n1 — n3, n1 — n4. n1 is the only AP, and every spoke is
    // a bridge.
    let (shared, nodes) = build_graph(4, &[(0, 1), (0, 2), (0, 3)]);
    let proj = build_proj(&shared);
    assert_eq!(articulation_points(&proj), vec![nodes[0]]);
    assert_eq!(
        bridges(&proj),
        vec![
            (nodes[0], nodes[1]),
            (nodes[0], nodes[2]),
            (nodes[0], nodes[3]),
        ]
    );
}

#[test]
fn articulation_two_triangles_sharing_node() {
    // Two triangles {n1, n2, n3} and {n3, n4, n5} share node n3 → n3 is the
    // only AP, no bridges (every edge lies on a cycle).
    let (shared, nodes) = build_graph(5, &[(0, 1), (1, 2), (2, 0), (2, 3), (3, 4), (4, 2)]);
    let proj = build_proj(&shared);
    assert_eq!(articulation_points(&proj), vec![nodes[2]]);
    assert!(bridges(&proj).is_empty());
}

#[test]
fn articulation_two_triangles_joined_by_bridge() {
    // Two triangles {n1,n2,n3} and {n4,n5,n6} joined by single edge n3 - n4.
    // APs: {n3, n4}. Bridges: {(n3, n4)}.
    let (shared, nodes) = build_graph(6, &[(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3), (2, 3)]);
    let proj = build_proj(&shared);
    assert_eq!(articulation_points(&proj), vec![nodes[2], nodes[3]]);
    assert_eq!(bridges(&proj), vec![(nodes[2], nodes[3])]);
}

#[test]
fn articulation_disconnected_components_handled_independently() {
    // Component A: path n1 - n2 - n3 (n2 AP; two bridges).
    // Component B: triangle n4 - n5 - n6 (no AP; no bridges).
    let (shared, nodes) = build_graph(6, &[(0, 1), (1, 2), (3, 4), (4, 5), (5, 3)]);
    let proj = build_proj(&shared);
    assert_eq!(articulation_points(&proj), vec![nodes[1]]);
    assert_eq!(
        bridges(&proj),
        vec![(nodes[0], nodes[1]), (nodes[1], nodes[2])]
    );
}

#[test]
fn articulation_bridge_endpoints_canonicalized_min_max() {
    // Insert edges where source > target to ensure bridge endpoints are
    // emitted as (min, max) per §E12.
    let (shared, nodes) = build_graph(2, &[(1, 0)]);
    let proj = build_proj(&shared);
    let result = bridges(&proj);
    assert_eq!(result, vec![(nodes[0], nodes[1])]);
    // Source < target invariant.
    for &(s, t) in &result {
        assert!(s.get() < t.get(), "bridge endpoints must be (min, max)");
    }
}

proptest! {
    /// Determinism + canonicalization invariants for articulation_points and
    /// bridges over random graphs.
    #[test]
    fn articulation_bridges_invariants(
        edge_seed in 0u64..32u64,
        edge_count in 0usize..24usize,
    ) {
        let shared = SharedGraph::new(GraphId::new(1));
        let label = istr("N");
        let rel = istr("R");
        let mut txn = shared.begin_write();
        let mut nodes = Vec::with_capacity(8);
        for _ in 0..8 {
            let id = txn
                .mutator()
                .create_node(LabelSet::single(label), PropertyMap::new())
                .unwrap();
            nodes.push(id);
        }
        let mut state = edge_seed.wrapping_add(1);
        for _ in 0..edge_count {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let s = (state % 8) as usize;
            let t = ((state >> 8) % 8) as usize;
            txn.mutator()
                .create_edge(rel, nodes[s], nodes[t], PropertyMap::new())
                .unwrap();
        }
        txn.commit().unwrap();

        let proj = build_proj(&shared);
        let ap = articulation_points(&proj);
        let br = bridges(&proj);

        // AP list: sorted ASC, deduped.
        for w in ap.windows(2) {
            prop_assert!(w[0].get() < w[1].get(), "AP not strictly ascending");
        }
        // Bridges: each (s, t) has s < t, and the outer list is sorted ASC by (s, t).
        for &(s, t) in &br {
            prop_assert!(s.get() < t.get(), "bridge endpoints must be (min, max)");
        }
        for w in br.windows(2) {
            let lhs = (w[0].0.get(), w[0].1.get());
            let rhs = (w[1].0.get(), w[1].1.get());
            prop_assert!(lhs < rhs, "bridges not sorted ASC by (source, target)");
        }
        // Bridges are deduped.
        let unique: std::collections::HashSet<(u64, u64)> =
            br.iter().map(|&(s, t)| (s.get(), t.get())).collect();
        prop_assert_eq!(unique.len(), br.len(), "duplicate bridges emitted");
    }
}
