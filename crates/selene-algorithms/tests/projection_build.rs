//! Integration tests for `GraphProjection::build` covering the §H test bar.

use proptest::prelude::*;
use roaring::RoaringBitmap;
use selene_algorithms::{GraphProjection, ProjectionConfig};
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_graph::SharedGraph;

fn istr(name: &str) -> IStr {
    intern(name).unwrap()
}

fn weight_props(value: f64) -> PropertyMap {
    PropertyMap::from_pairs([(istr("weight"), Value::Float(value))]).unwrap()
}

/// Build a small fixture:
///   Person(1) -- friend(weight=2.5) --> Person(2)
///   Person(2) -- friend(weight=4.0) --> Company(3)
///   Company(3) -- employs(weight=missing) --> Person(1)
fn fixture_small() -> (SharedGraph, [NodeId; 3], [IStr; 2]) {
    let shared = SharedGraph::new(GraphId::new(1));
    let person = istr("Person");
    let company = istr("Company");
    let friend = istr("friend");
    let employs = istr("employs");
    let weight = istr("weight");

    let mut txn = shared.begin_write();
    let n1 = txn
        .mutator()
        .create_node(LabelSet::single(person), PropertyMap::new())
        .unwrap();
    let n2 = txn
        .mutator()
        .create_node(LabelSet::single(person), PropertyMap::new())
        .unwrap();
    let n3 = txn
        .mutator()
        .create_node(LabelSet::single(company), PropertyMap::new())
        .unwrap();

    txn.mutator()
        .create_edge(friend, n1, n2, weight_props(2.5))
        .unwrap();
    txn.mutator()
        .create_edge(friend, n2, n3, weight_props(4.0))
        .unwrap();
    // Edge with no weight property — exercises §O.2 default-to-1.0.
    txn.mutator()
        .create_edge(employs, n3, n1, PropertyMap::new())
        .unwrap();

    txn.commit().unwrap();
    let _ = weight;
    (shared, [n1, n2, n3], [friend, employs])
}

#[test]
fn projection_all_nodes_all_edges() {
    let (shared, nodes, _) = fixture_small();
    let snapshot = shared.read();
    let proj = GraphProjection::build(
        &snapshot,
        &ProjectionConfig {
            name: "all".to_string(),
            node_labels: vec![],
            edge_labels: vec![],
            weight_property: None,
        },
        None,
    )
    .unwrap();

    assert_eq!(proj.node_count(), 3);
    assert_eq!(proj.edge_count(), 3, "all 3 directed edges present");
    assert_eq!(proj.out_neighbors(nodes[0]).len(), 1);
    assert_eq!(proj.in_neighbors(nodes[0]).len(), 1);
    assert!(proj.contains(nodes[0]));
    assert!(!proj.is_weighted());
}

#[test]
fn projection_label_filter() {
    let (shared, nodes, _) = fixture_small();
    let snapshot = shared.read();
    let person = istr("Person");
    let proj = GraphProjection::build(
        &snapshot,
        &ProjectionConfig {
            name: "people".to_string(),
            node_labels: vec![person],
            edge_labels: vec![],
            weight_property: None,
        },
        None,
    )
    .unwrap();

    assert_eq!(proj.node_count(), 2, "only 2 Person nodes");
    assert!(proj.contains(nodes[0]));
    assert!(proj.contains(nodes[1]));
    assert!(!proj.contains(nodes[2]), "Company node excluded");
    // The only edge between Person nodes is n1 -> n2 (friend).
    assert_eq!(proj.edge_count(), 1);
    assert_eq!(proj.out_neighbors(nodes[0])[0].node_id, nodes[1]);
}

#[test]
fn projection_edge_label_filter() {
    let (shared, _, labels) = fixture_small();
    let snapshot = shared.read();
    let [friend, _] = labels;
    let proj = GraphProjection::build(
        &snapshot,
        &ProjectionConfig {
            name: "friends".to_string(),
            node_labels: vec![],
            edge_labels: vec![friend],
            weight_property: None,
        },
        None,
    )
    .unwrap();

    assert_eq!(
        proj.edge_count(),
        2,
        "only 2 friend-typed edges; employs filtered out"
    );
    assert_eq!(proj.edge_labels(), &[friend]);
}

#[test]
fn projection_transpose_invariant_holds_on_directed_dag() {
    let shared = SharedGraph::new(GraphId::new(96_001));
    let label = istr("N");
    let rel = istr("R");

    let mut txn = shared.begin_write();
    let mut nodes = Vec::with_capacity(4);
    for _ in 0..4 {
        nodes.push(
            txn.mutator()
                .create_node(LabelSet::single(label), PropertyMap::new())
                .unwrap(),
        );
    }
    for (source, target) in [(0, 1), (1, 2), (2, 3)] {
        txn.mutator()
            .create_edge(rel, nodes[source], nodes[target], PropertyMap::new())
            .unwrap();
    }
    txn.commit().unwrap();

    let snapshot = shared.read();
    let proj = GraphProjection::build(
        &snapshot,
        &ProjectionConfig {
            name: "directed-dag".to_string(),
            node_labels: vec![],
            edge_labels: vec![],
            weight_property: None,
        },
        None,
    )
    .unwrap();

    assert_eq!(proj.node_count(), 4);
    assert_eq!(proj.edge_count(), 3);
    assert_eq!(proj.out_neighbors(nodes[0])[0].node_id, nodes[1]);
    assert_eq!(proj.in_neighbors(nodes[3])[0].node_id, nodes[2]);
}

#[test]
fn projection_transpose_invariant_holds_on_label_filtered_non_reciprocal() {
    let shared = SharedGraph::new(GraphId::new(96_002));
    let label = istr("N");
    let knows = istr("KNOWS");
    let owns = istr("OWNS");

    let mut txn = shared.begin_write();
    let a = txn
        .mutator()
        .create_node(LabelSet::single(label), PropertyMap::new())
        .unwrap();
    let b = txn
        .mutator()
        .create_node(LabelSet::single(label), PropertyMap::new())
        .unwrap();
    let c = txn
        .mutator()
        .create_node(LabelSet::single(label), PropertyMap::new())
        .unwrap();
    let d = txn
        .mutator()
        .create_node(LabelSet::single(label), PropertyMap::new())
        .unwrap();
    txn.mutator()
        .create_edge(knows, a, b, PropertyMap::new())
        .unwrap();
    txn.mutator()
        .create_edge(owns, c, d, PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();

    let snapshot = shared.read();
    let proj = GraphProjection::build(
        &snapshot,
        &ProjectionConfig {
            name: "knows-only".to_string(),
            node_labels: vec![],
            edge_labels: vec![knows],
            weight_property: None,
        },
        None,
    )
    .unwrap();

    assert_eq!(proj.node_count(), 4);
    assert_eq!(proj.edge_count(), 1);
    assert_eq!(proj.out_neighbors(a)[0].node_id, b);
    assert_eq!(proj.in_neighbors(b)[0].node_id, a);
    assert!(proj.out_neighbors(b).is_empty(), "KNOWS edge is one-way");
    assert!(
        proj.out_neighbors(c).is_empty(),
        "OWNS edge is filtered out"
    );
    assert!(proj.in_neighbors(d).is_empty(), "OWNS edge is filtered out");
}

#[test]
fn projection_weight_extraction() {
    let (shared, nodes, _) = fixture_small();
    let snapshot = shared.read();
    let weight = istr("weight");
    let proj = GraphProjection::build(
        &snapshot,
        &ProjectionConfig {
            name: "weighted".to_string(),
            node_labels: vec![],
            edge_labels: vec![],
            weight_property: Some(weight),
        },
        None,
    )
    .unwrap();

    assert!(proj.is_weighted());
    let out_n1 = proj.out_neighbors(nodes[0]);
    assert_eq!(out_n1.len(), 1);
    assert_eq!(out_n1[0].weight, 2.5);

    let out_n2 = proj.out_neighbors(nodes[1]);
    assert_eq!(out_n2.len(), 1);
    assert_eq!(out_n2[0].weight, 4.0);
}

#[test]
fn projection_weight_missing_defaults_to_one() {
    // The employs edge n3 -> n1 carries no `weight` property; per spec 16
    // §E04 (and §O.2) the projection MUST default to 1.0, not error.
    let (shared, nodes, _) = fixture_small();
    let snapshot = shared.read();
    let weight = istr("weight");
    let proj = GraphProjection::build(
        &snapshot,
        &ProjectionConfig {
            name: "weighted-mixed".to_string(),
            node_labels: vec![],
            edge_labels: vec![],
            weight_property: Some(weight),
        },
        None,
    )
    .unwrap();

    let out_n3 = proj.out_neighbors(nodes[2]);
    assert_eq!(out_n3.len(), 1, "n3 has one outgoing edge (employs)");
    assert_eq!(
        out_n3[0].weight, 1.0,
        "missing weight property defaults to 1.0 per spec 16 §E04"
    );
    // The other edges still carry their declared weights.
    assert_eq!(proj.out_neighbors(nodes[0])[0].weight, 2.5);
    assert_eq!(proj.out_neighbors(nodes[1])[0].weight, 4.0);
}

#[test]
fn projection_scope_intersect() {
    let (shared, nodes, _) = fixture_small();
    let snapshot = shared.read();
    // Scope bitmap restricts to rows 0 and 1 only (NodeId 1 and 2).
    let mut scope = RoaringBitmap::new();
    scope.insert(0);
    scope.insert(1);

    let proj = GraphProjection::build(
        &snapshot,
        &ProjectionConfig {
            name: "scoped".to_string(),
            node_labels: vec![],
            edge_labels: vec![],
            weight_property: None,
        },
        Some(&scope),
    )
    .unwrap();

    assert_eq!(proj.node_count(), 2);
    assert!(proj.contains(nodes[0]));
    assert!(proj.contains(nodes[1]));
    assert!(!proj.contains(nodes[2]));
    // Only the n1 -> n2 friend edge survives the scope intersection.
    assert_eq!(proj.edge_count(), 1);
}

#[test]
fn projection_generation_pinned() {
    let (shared, _, _) = fixture_small();
    let snapshot_v1 = shared.read();
    let gen_v1 = snapshot_v1.meta.generation;
    let proj = GraphProjection::build(
        &snapshot_v1,
        &ProjectionConfig {
            name: "pinned".to_string(),
            node_labels: vec![],
            edge_labels: vec![],
            weight_property: None,
        },
        None,
    )
    .unwrap();
    assert_eq!(proj.generation(), gen_v1);
    drop(snapshot_v1);

    // Mutate the graph: append one more Person node and commit.
    let mut txn = shared.begin_write();
    let person = istr("Person");
    txn.mutator()
        .create_node(LabelSet::single(person), PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();

    let snapshot_v2 = shared.read();
    assert!(
        snapshot_v2.meta.generation > gen_v1,
        "generation should advance after a mutating commit"
    );
    // The projection was built against the v1 snapshot; its generation must
    // remain at gen_v1 (frozen at build time per spec 16 §E02).
    assert_eq!(
        proj.generation(),
        gen_v1,
        "projection generation is frozen at build time per §E02"
    );
}

proptest! {
    /// For any projection over a randomly-built graph, `out_neighbors(n)` MUST
    /// be sorted ASC by `node_id` for every `n` per spec 16 §E03.
    #[test]
    fn projection_neighbor_ordering_invariant(
        edge_seed in 0u64..8u64,
        edge_count in 0usize..32usize,
    ) {
        let shared = SharedGraph::new(GraphId::new(1));
        let label = istr("N");
        let rel = istr("R");

        let mut txn = shared.begin_write();
        let mut nodes: Vec<NodeId> = Vec::with_capacity(8);
        for _ in 0..8 {
            let id = txn
                .mutator()
                .create_node(LabelSet::single(label), PropertyMap::new())
                .unwrap();
            nodes.push(id);
        }
        let mut state = edge_seed.wrapping_add(1);
        for _ in 0..edge_count {
            // Deterministic xorshift-like step so the property test inputs map
            // onto reproducible edge layouts.
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let src_idx = (state % 8) as usize;
            let tgt_idx = ((state >> 8) % 8) as usize;
            txn
                .mutator()
                .create_edge(rel, nodes[src_idx], nodes[tgt_idx], PropertyMap::new())
                .unwrap();
        }
        txn.commit().unwrap();

        let snapshot = shared.read();
        let proj = GraphProjection::build(
            &snapshot,
            &ProjectionConfig {
                name: "proptest".to_string(),
                node_labels: vec![],
                edge_labels: vec![],
                weight_property: None,
            },
            None,
        )
        .unwrap();

        for n in nodes.iter().copied() {
            let out = proj.out_neighbors(n);
            for window in out.windows(2) {
                prop_assert!(
                    window[0].node_id <= window[1].node_id,
                    "out_neighbors not sorted ASC: {:?} > {:?}",
                    window[0].node_id,
                    window[1].node_id,
                );
            }
            let inc = proj.in_neighbors(n);
            for window in inc.windows(2) {
                prop_assert!(
                    window[0].node_id <= window[1].node_id,
                    "in_neighbors not sorted ASC: {:?} > {:?}",
                    window[0].node_id,
                    window[1].node_id,
                );
            }
        }

    }
}
