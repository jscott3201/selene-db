//! GRAPH-05: holistic hub-delete cascade.
//!
//! Deleting a degree-`D` node drops the hub's own adjacency entries wholesale
//! and clears only the *neighbor* side of each incident edge in place (O(D),
//! not the old O(D^2) clone-and-scan-per-edge). These tests pin the observable
//! invariants of that path: every incident edge removed, the hub's entries
//! gone, neighbors still alive with no dangling back-edge to the dead hub, and
//! — the load-bearing guard — a neighbor's *unrelated* edges left untouched by
//! the wholesale drop.

use selene_core::{GraphId, PropertyMap, db_string};

use super::*;
use crate::SharedGraph;

/// Build a star: one hub joined to `n` leaves by BOTH a `hub -> leaf` out-edge
/// and a `leaf -> hub` in-edge, so the cascade exercises both adjacency
/// directions. Returns `(hub, leaves, every incident edge id)`.
fn build_star(mutator: &mut Mutator<'_, '_>, n: usize) -> (NodeId, Vec<NodeId>, Vec<EdgeId>) {
    let out_label = db_string("hub.out").unwrap();
    let in_label = db_string("hub.in").unwrap();
    let hub = mutator
        .create_node(LabelSet::new(), PropertyMap::new())
        .unwrap();
    let mut leaves = Vec::with_capacity(n);
    let mut edges = Vec::with_capacity(2 * n);
    for _ in 0..n {
        let leaf = mutator
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        edges.push(
            mutator
                .create_edge(out_label.clone(), hub, leaf, PropertyMap::new())
                .unwrap(),
        );
        edges.push(
            mutator
                .create_edge(in_label.clone(), leaf, hub, PropertyMap::new())
                .unwrap(),
        );
        leaves.push(leaf);
    }
    (hub, leaves, edges)
}

#[test]
fn delete_hub_clears_all_incident_edges_and_hub_entries() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (hub, leaves, edges) = {
        let mut mutator = txn.mutator();
        let (hub, leaves, edges) = build_star(&mut mutator, 32);
        mutator.delete_node(hub).unwrap();
        (hub, leaves, edges)
    };
    txn.commit().unwrap();
    let snapshot = shared.read();

    assert!(!snapshot.is_node_alive(hub));
    // Hub's own adjacency entries were dropped wholesale (not edge-by-edge).
    assert!(snapshot.outgoing_edges(hub).is_none());
    assert!(snapshot.incoming_edges(hub).is_none());
    // Every incident edge is gone.
    for edge in edges {
        assert!(
            !snapshot.is_edge_alive(edge),
            "incident edge {edge:?} should be deleted"
        );
    }
    // Every leaf survives; each had edges *only* to the hub, so its adjacency
    // entries are now empty and therefore removed (the no-empty-entry invariant).
    for leaf in leaves {
        assert!(snapshot.is_node_alive(leaf));
        assert!(
            snapshot.outgoing_edges(leaf).is_none(),
            "leaf {leaf:?} kept a dangling out back-edge to the dead hub"
        );
        assert!(
            snapshot.incoming_edges(leaf).is_none(),
            "leaf {leaf:?} kept a dangling in back-edge from the dead hub"
        );
    }
}

#[test]
fn delete_hub_preserves_neighbor_unrelated_edges() {
    // The wholesale hub-entry drop must leave a neighbor's edges that do not
    // involve the hub completely intact.
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (hub, l0, l1, hub_edge, side_edge) = {
        let mut mutator = txn.mutator();
        let hub = mutator
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        let l0 = mutator
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        let l1 = mutator
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        // hub -> l0 (incident on the hub)
        let hub_edge = mutator
            .create_edge(db_string("hub.link").unwrap(), hub, l0, PropertyMap::new())
            .unwrap();
        // l0 -> l1 (does NOT involve the hub)
        let side_edge = mutator
            .create_edge(db_string("side.link").unwrap(), l0, l1, PropertyMap::new())
            .unwrap();
        mutator.delete_node(hub).unwrap();
        (hub, l0, l1, hub_edge, side_edge)
    };
    txn.commit().unwrap();
    let snapshot = shared.read();

    assert!(!snapshot.is_node_alive(hub));
    assert!(!snapshot.is_edge_alive(hub_edge));
    assert!(snapshot.outgoing_edges(hub).is_none());

    // The unrelated l0 -> l1 edge survives untouched.
    assert!(snapshot.is_edge_alive(side_edge));
    let l0_out = snapshot.outgoing_edges(l0).expect("l0 keeps its side edge");
    assert_eq!(
        l0_out.len(),
        1,
        "only the hub back-edge should have been removed"
    );
    assert_eq!(l0_out.iter().next().unwrap().edge_id, side_edge);
    // l0's only incoming edge was from the hub -> now empty -> entry removed.
    assert!(snapshot.incoming_edges(l0).is_none());
    // l1 keeps its incoming side edge.
    let l1_in = snapshot
        .incoming_edges(l1)
        .expect("l1 keeps its incoming side edge");
    assert_eq!(l1_in.len(), 1);
    assert_eq!(l1_in.iter().next().unwrap().edge_id, side_edge);
}
