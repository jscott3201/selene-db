//! BRIEF-Item-4a external-id <-> [`RowIndex`](crate::store::RowIndex) mapping
//! tests, kept separate from `mutator/tests.rs` to stay under the 700-LOC cap.
//!
//! Increment 2 lands the population divergence guard; Increment 6 adds the
//! non-identity proof test (a manually constructed map where `id != row + 1`).

use selene_core::{GraphId, LabelSet, NodeId, PropertyMap};

use crate::SharedGraph;
use crate::store::{RowIndex, edge_row_index, node_row_index};

#[test]
fn id_row_maps_agree_with_arithmetic_for_all_alive() {
    // BRIEF-Item-4a Increment 2 divergence guard. The populated id<->row maps
    // and per-store `row_to_id` columns must agree with the legacy
    // `id == row + 1` arithmetic for every ALIVE node and edge, and a deleted id
    // must stay mapped (so a later map-backed read yields NotAlive, not
    // NotFound). Reads are still arithmetic in Increment 2, so any divergence
    // here is a population bug surfacing before Increment 3 flips reads onto the
    // maps. Bar: "would this catch the IStr admission race" — it walks every
    // alive row through both directions plus the delete-tombstone path.
    let shared = SharedGraph::new(GraphId::new(1));
    let a = selene_core::intern("inc2.a").unwrap();
    let b = selene_core::intern("inc2.b").unwrap();
    let mut txn = shared.begin_write();
    {
        let mut m = txn.mutator();
        let n0 = m
            .create_node(LabelSet::single(a), PropertyMap::new())
            .unwrap();
        let n1 = m
            .create_node(LabelSet::single(a), PropertyMap::new())
            .unwrap();
        let n2 = m
            .create_node(LabelSet::single(b), PropertyMap::new())
            .unwrap();
        // e0/e1 are incident to n1 (cascade-deleted); e2 (n0->n2) survives.
        m.create_edge(a, n0, n1, PropertyMap::new()).unwrap();
        m.create_edge(a, n1, n2, PropertyMap::new()).unwrap();
        m.create_edge(a, n0, n2, PropertyMap::new()).unwrap();
        m.delete_node(n1).unwrap();
    }
    txn.commit().unwrap();
    let g = shared.read();

    // Every alive node row round-trips and equals the arithmetic mapping.
    for row in g.node_store.alive.iter() {
        let id = *g
            .node_store
            .row_to_id
            .get(row as usize)
            .expect("row_to_id in-bounds");
        assert_ne!(
            id,
            NodeId::TOMBSTONE,
            "alive node row {row} has tombstone id"
        );
        assert_eq!(
            node_row_index(id),
            Some(row),
            "arith disagrees for alive {id}"
        );
        assert_eq!(g.node_id_to_row.get(&id).copied(), Some(RowIndex::new(row)));
    }
    // n1 == row 1 == NodeId(2): deleted, but its id stays mapped to the dead row
    // AND its row_to_id slot keeps the real id (Option B) so the snapshot/STEP-9
    // encoder persists the dead row's id and recovery rebuilds NotAlive (not
    // NotFound) for it.
    assert!(!g.node_store.alive.contains(1));
    assert_eq!(
        g.node_id_to_row.get(&NodeId::new(2)).copied(),
        Some(RowIndex::new(1)),
        "deleted node id must stay mapped (NotAlive, not NotFound)"
    );
    assert_eq!(
        *g.node_store.row_to_id.get(1).unwrap(),
        NodeId::new(2),
        "deleted row keeps its real external id in row_to_id"
    );

    // Every alive edge row round-trips; e2 (row 2 == EdgeId(3)) is the survivor.
    for row in g.edge_store.alive.iter() {
        let id = *g
            .edge_store
            .row_to_id
            .get(row as usize)
            .expect("row_to_id in-bounds");
        assert_ne!(
            id,
            selene_core::EdgeId::TOMBSTONE,
            "alive edge row {row} has tombstone id"
        );
        assert_eq!(
            edge_row_index(id),
            Some(row),
            "arith disagrees for alive {id}"
        );
        assert_eq!(g.edge_id_to_row.get(&id).copied(), Some(RowIndex::new(row)));
    }
    assert!(g.edge_store.alive.contains(2));
    // Cascade-deleted edge ids stay mapped to their dead rows, and their
    // row_to_id slots keep the real id (Option B), same as deleted nodes.
    assert_eq!(
        g.edge_id_to_row.get(&selene_core::EdgeId::new(1)).copied(),
        Some(RowIndex::new(0))
    );
    assert_eq!(
        g.edge_id_to_row.get(&selene_core::EdgeId::new(2)).copied(),
        Some(RowIndex::new(1))
    );
    assert_eq!(
        *g.edge_store.row_to_id.get(0).unwrap(),
        selene_core::EdgeId::new(1)
    );
    assert_eq!(
        *g.edge_store.row_to_id.get(1).unwrap(),
        selene_core::EdgeId::new(2)
    );

    // The row_to_id columns track the row columns exactly (length-locked).
    assert_eq!(g.node_store.row_to_id.len(), g.node_store.len());
    assert_eq!(g.edge_store.row_to_id.len(), g.edge_store.len());
}
