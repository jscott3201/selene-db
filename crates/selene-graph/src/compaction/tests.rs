//! BRIEF-Item-4b CORE compaction tests.
//!
//! Bar ("would this catch the IStr admission race"): the suite drives a graph
//! with a cascade-deleting node delete through `compact_core`, then walks every
//! surviving external id through every read path (labels, properties,
//! endpoints, adjacency, label index) asserting observational equivalence,
//! plus the dense-layout / NotFound-for-reclaimed / preserved-high-water-mark
//! invariants that a remap bug would break.

use std::collections::HashSet;

use selene_core::{EdgeId, GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};

use super::{compact_core, live_id_set};
use crate::store::RowIndex;
use crate::{AdjacencyEntry, CompactionReport, SeleneGraph, SharedGraph};

fn prop(key: &str, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(intern(key).unwrap(), value)]).unwrap()
}

/// Sorted (neighbor, edge_id, label) projection of an adjacency entry, for
/// order-independent equality across the row renumber.
fn adjacency_summary(entry: &AdjacencyEntry) -> Vec<(NodeId, EdgeId, IStr)> {
    let mut edges: Vec<(NodeId, EdgeId, IStr)> = entry
        .iter()
        .map(|edge| (edge.neighbor, edge.edge_id, edge.label))
        .collect();
    edges.sort_by_key(|(neighbor, edge_id, _)| (*neighbor, *edge_id));
    edges
}

/// 4 nodes (ids 1..=4), 3 edges (ids 1..=3), then `delete_node(2)` which
/// cascade-deletes its incident edge (id 2). Surviving: nodes 1,3,4; edges 1,3.
fn graph_with_a_deletion() -> SharedGraph {
    let shared = SharedGraph::new(GraphId::new(1));
    let la = intern("cmp.a").unwrap();
    let lb = intern("cmp.b").unwrap();
    let el = intern("cmp.e").unwrap();
    let mut txn = shared.begin_write();
    {
        let mut m = txn.mutator();
        let n1 = m
            .create_node(LabelSet::single(la), prop("name", Value::Int(1)))
            .unwrap();
        let n2 = m
            .create_node(LabelSet::single(lb), prop("name", Value::Int(2)))
            .unwrap();
        let n3 = m
            .create_node(LabelSet::single(la), prop("name", Value::Int(3)))
            .unwrap();
        let n4 = m
            .create_node(LabelSet::single(la), prop("name", Value::Int(4)))
            .unwrap();
        assert_eq!(
            (n1, n2, n3, n4),
            (
                NodeId::new(1),
                NodeId::new(2),
                NodeId::new(3),
                NodeId::new(4)
            )
        );
        m.create_edge(el, n1, n3, prop("w", Value::Int(10)))
            .unwrap(); // id 1 — survives
        m.create_edge(el, n1, n2, PropertyMap::new()).unwrap(); // id 2 — cascade-deleted
        m.create_edge(el, n3, n4, PropertyMap::new()).unwrap(); // id 3 — survives
        m.delete_node(n2).unwrap();
    }
    txn.commit().unwrap();
    shared
}

#[test]
fn compaction_drops_dead_rows_and_renumbers_dense() {
    let shared = graph_with_a_deletion();
    let before = shared.read();
    let next_node_before = before.meta.next_node_id;
    let next_edge_before = before.meta.next_edge_id;
    let live_nodes = before.node_count();
    let len_before = before.node_store.len();

    let compacted = compact_core(&before).unwrap();
    let g = &compacted.graph;

    // Dense: every row alive, no holes, len == live count.
    assert_eq!(g.node_store.len(), live_nodes);
    assert!(
        len_before > g.node_store.len(),
        "dead rows existed to reclaim"
    );
    assert_eq!(g.node_count(), live_nodes);
    for row in 0..g.node_store.len() as u32 {
        let id = g
            .node_id_for_row(RowIndex::new(row))
            .expect("every dense row has a live external id");
        assert!(g.is_node_alive(id), "dense row {row} ({id}) must be alive");
    }
    assert_eq!(
        compacted.report.reclaimed_nodes,
        (len_before - g.node_store.len()) as u64
    );
    assert!(
        compacted.report.reclaimed_edges >= 1,
        "the n1->n2 edge (id 2) was reclaimed"
    );

    // Survivors renumber dense in ascending old-row order: 1@old0->new0,
    // 3@old2->new1, 4@old3->new2 (old row 1, node 2, is gone).
    assert_eq!(g.row_for_node_id(NodeId::new(1)), Some(RowIndex::new(0)));
    assert_eq!(g.row_for_node_id(NodeId::new(3)), Some(RowIndex::new(1)));
    assert_eq!(g.row_for_node_id(NodeId::new(4)), Some(RowIndex::new(2)));

    // The reclaimed deleted id flips NotAlive -> NotFound (the deliberate
    // compaction semantic): pre-compaction it stayed mapped to its dead row;
    // post-compaction the row is gone and the id resolves to nothing.
    assert!(!before.is_node_alive(NodeId::new(2)));
    assert!(
        before.row_for_node_id(NodeId::new(2)).is_some(),
        "pre-compaction: deleted id still mapped (NotAlive, Option B)"
    );
    assert!(
        g.row_for_node_id(NodeId::new(2)).is_none(),
        "post-compaction: deleted id reclaimed (NotFound)"
    );
    assert!(!g.is_node_alive(NodeId::new(2)));

    // Allocator high-water marks preserved verbatim — no external id reuse.
    assert_eq!(g.meta.next_node_id, next_node_before);
    assert_eq!(g.meta.next_edge_id, next_edge_before);
}

#[test]
fn compaction_preserves_observable_reads() {
    let shared = graph_with_a_deletion();
    let before = shared.read();
    let compacted = compact_core(&before).unwrap();
    let g = &compacted.graph;

    for id in [NodeId::new(1), NodeId::new(3), NodeId::new(4)] {
        assert_eq!(g.node_labels(id), before.node_labels(id), "labels {id}");
        assert_eq!(
            g.node_properties(id),
            before.node_properties(id),
            "properties {id}"
        );
        assert_eq!(
            g.outgoing_edges(id).map(adjacency_summary),
            before.outgoing_edges(id).map(adjacency_summary),
            "outgoing adjacency {id}"
        );
        assert_eq!(
            g.incoming_edges(id).map(adjacency_summary),
            before.incoming_edges(id).map(adjacency_summary),
            "incoming adjacency {id}"
        );
    }
    for id in [EdgeId::new(1), EdgeId::new(3)] {
        assert_eq!(
            g.edge_endpoints(id),
            before.edge_endpoints(id),
            "endpoints {id}"
        );
        assert_eq!(g.edge_label(id), before.edge_label(id), "edge label {id}");
        assert_eq!(
            g.edge_properties(id),
            before.edge_properties(id),
            "edge properties {id}"
        );
    }

    // The label index resolves the SAME external ids after the row renumber.
    let la = intern("cmp.a").unwrap();
    let external_with_label = |graph: &SeleneGraph| -> HashSet<NodeId> {
        graph
            .nodes_with_label(&la)
            .map(|bitmap| {
                bitmap
                    .iter()
                    .map(|row| graph.node_id_for_row(RowIndex::new(row)).unwrap())
                    .collect()
            })
            .unwrap_or_default()
    };
    assert_eq!(external_with_label(g), external_with_label(&before));
    assert_eq!(
        external_with_label(g),
        HashSet::from([NodeId::new(1), NodeId::new(3), NodeId::new(4)])
    );
}

#[test]
fn live_id_set_reflects_alive_external_ids() {
    let shared = graph_with_a_deletion();
    let live = live_id_set(&shared.read());
    assert_eq!(
        live.nodes,
        HashSet::from([NodeId::new(1), NodeId::new(3), NodeId::new(4)])
    );
    assert!(!live.contains_node(NodeId::new(2)));
    assert!(live.contains_edge(EdgeId::new(1)) && live.contains_edge(EdgeId::new(3)));
    assert!(!live.contains_edge(EdgeId::new(2)), "cascade-deleted edge");
}

#[test]
fn compacted_graph_republishes_cleanly() {
    // The compacted graph is a valid publishable graph: SharedGraph::from_graph
    // re-runs the full rebuild + (debug) consistency assertion, and every read
    // survives the round-trip. This is the 4c-readiness proof.
    let shared = graph_with_a_deletion();
    let compacted = compact_core(&shared.read()).unwrap();
    let republished = SharedGraph::from_graph(compacted.graph);
    let g = republished.read();

    assert_eq!(g.node_count(), 3);
    assert_eq!(g.edge_count(), 2);
    assert!(g.is_node_alive(NodeId::new(1)));
    assert!(g.row_for_node_id(NodeId::new(2)).is_none());
    assert_eq!(
        g.edge_endpoints(EdgeId::new(1)),
        Some((NodeId::new(1), NodeId::new(3)))
    );
    assert_eq!(
        g.edge_endpoints(EdgeId::new(3)),
        Some((NodeId::new(3), NodeId::new(4)))
    );
}

#[test]
fn compacting_a_dense_graph_is_idempotent() {
    // A graph with no dead rows compacts to an observationally identical graph
    // (same ids, same dense rows) and reclaims nothing.
    let shared = SharedGraph::new(GraphId::new(1));
    let la = intern("cmp.idem").unwrap();
    let mut txn = shared.begin_write();
    {
        let mut m = txn.mutator();
        m.create_node(LabelSet::single(la), PropertyMap::new())
            .unwrap();
        m.create_node(LabelSet::single(la), PropertyMap::new())
            .unwrap();
    }
    txn.commit().unwrap();

    let before = shared.read();
    let compacted = compact_core(&before).unwrap();
    let g = &compacted.graph;
    assert_eq!(compacted.report, CompactionReport::default());
    assert_eq!(g.node_store.len(), before.node_store.len());
    // Every external id keeps its exact row (nothing to renumber), asserted per
    // id against the explicit dense layout — not by a single survivor that
    // happened not to move.
    for id in [NodeId::new(1), NodeId::new(2)] {
        assert_eq!(
            g.row_for_node_id(id),
            before.row_for_node_id(id),
            "dense graph: {id} must keep its row"
        );
    }
    assert_eq!(g.row_for_node_id(NodeId::new(1)), Some(RowIndex::new(0)));
    assert_eq!(g.row_for_node_id(NodeId::new(2)), Some(RowIndex::new(1)));
    assert_eq!(g.meta.next_node_id, before.meta.next_node_id);
}

#[test]
fn compaction_of_empty_graph_is_a_clean_noop() {
    let shared = SharedGraph::new(GraphId::new(1));
    let before = shared.read();
    let compacted = compact_core(&before).unwrap();

    assert_eq!(compacted.graph.node_store.len(), 0);
    assert_eq!(compacted.graph.edge_store.len(), 0);
    assert_eq!(compacted.graph.node_count(), 0);
    assert_eq!(compacted.report, CompactionReport::default());
    assert!(compacted.live.nodes.is_empty() && compacted.live.edges.is_empty());
    // High-water marks of a virgin allocator carry through untouched.
    assert_eq!(compacted.graph.meta.next_node_id, before.meta.next_node_id);
    // An empty compacted graph is still a valid publishable graph.
    let _ = SharedGraph::from_graph(compacted.graph);
}

#[test]
fn compaction_of_an_all_deleted_graph_reclaims_everything() {
    // Every row dead: compaction reclaims all of them, the result is empty, and
    // the allocator high-water marks are still preserved so the deleted ids are
    // never reissued by a later create.
    let shared = SharedGraph::new(GraphId::new(1));
    let la = intern("cmp.alldel").unwrap();
    let el = intern("cmp.alldele").unwrap();
    let mut txn = shared.begin_write();
    {
        let mut m = txn.mutator();
        let n1 = m
            .create_node(LabelSet::single(la), PropertyMap::new())
            .unwrap();
        let n2 = m
            .create_node(LabelSet::single(la), PropertyMap::new())
            .unwrap();
        m.create_edge(el, n1, n2, PropertyMap::new()).unwrap();
        m.delete_node(n1).unwrap(); // cascade-deletes the edge
        m.delete_node(n2).unwrap();
    }
    txn.commit().unwrap();

    let before = shared.read();
    assert_eq!(before.node_count(), 0);
    let rows_before = before.node_store.len() as u64;
    let next_node_before = before.meta.next_node_id;
    let next_edge_before = before.meta.next_edge_id;

    let compacted = compact_core(&before).unwrap();
    let g = &compacted.graph;

    assert_eq!(g.node_store.len(), 0);
    assert_eq!(g.edge_store.len(), 0);
    assert_eq!(g.node_count(), 0);
    assert_eq!(compacted.report.reclaimed_nodes, rows_before);
    assert!(compacted.report.reclaimed_edges >= 1);
    assert!(g.row_for_node_id(NodeId::new(1)).is_none());
    assert!(g.row_for_node_id(NodeId::new(2)).is_none());
    // No id reuse: the deleted ids stay burned.
    assert_eq!(g.meta.next_node_id, next_node_before);
    assert_eq!(g.meta.next_edge_id, next_edge_before);
    let _ = SharedGraph::from_graph(compacted.graph);
}

#[test]
fn compaction_reclaims_an_aborted_tx_interior_hole() {
    // Burn id 2 with an aborted txn, then materialize the hole row by creating
    // id 3 (whose arith row lands past the gap). The canonical store now holds
    // an interior TOMBSTONE row that was NEVER alive. Compaction must drop it,
    // leave id 2 NotFound, and preserve next_node_id so the burned id is never
    // reissued.
    let shared = SharedGraph::new(GraphId::new(1));
    let la = intern("cmp.hole").unwrap();

    let mut txn = shared.begin_write();
    {
        let mut m = txn.mutator();
        let n1 = m
            .create_node(LabelSet::single(la), PropertyMap::new())
            .unwrap();
        assert_eq!(n1, NodeId::new(1));
    }
    txn.commit().unwrap();

    // Aborted txn: allocate id 2, then drop without commit — id burned, no row.
    {
        let mut txn = shared.begin_write();
        let mut m = txn.mutator();
        let n2 = m
            .create_node(LabelSet::single(la), PropertyMap::new())
            .unwrap();
        assert_eq!(n2, NodeId::new(2));
        // txn dropped here, uncommitted.
    }

    // Materialize the interior hole: id 3's arith row (index 2) pads row 1.
    let mut txn = shared.begin_write();
    {
        let mut m = txn.mutator();
        let n3 = m
            .create_node(LabelSet::single(la), PropertyMap::new())
            .unwrap();
        assert_eq!(n3, NodeId::new(3));
    }
    txn.commit().unwrap();

    let before = shared.read();
    assert_eq!(before.node_count(), 2, "n1 + n3 alive; id 2 burned");
    assert!(
        before.row_for_node_id(NodeId::new(2)).is_none(),
        "aborted id was never committed -> NotFound even pre-compaction"
    );
    let rows_before = before.node_store.len() as u64;
    assert!(rows_before >= 3, "an interior hole row exists to reclaim");
    let next_node_before = before.meta.next_node_id;

    let compacted = compact_core(&before).unwrap();
    let g = &compacted.graph;

    assert_eq!(g.node_store.len(), 2, "the hole row is reclaimed");
    assert_eq!(g.node_count(), 2);
    assert_eq!(g.row_for_node_id(NodeId::new(1)), Some(RowIndex::new(0)));
    assert_eq!(g.row_for_node_id(NodeId::new(3)), Some(RowIndex::new(1)));
    assert!(g.row_for_node_id(NodeId::new(2)).is_none());
    assert!(compacted.report.reclaimed_nodes >= 1);
    assert_eq!(
        g.meta.next_node_id, next_node_before,
        "burned id 2 must never be reissued"
    );
    assert!(!compacted.live.contains_node(NodeId::new(2)));
}

#[test]
fn republished_compacted_graph_allocates_without_reuse() {
    // The 4c-critical property: after a compacted graph goes live, the very next
    // create must allocate the PRESERVED next_node_id (not reuse a reclaimed id,
    // not alias a survivor's renumbered row). create_node still uses arith
    // row = id - 1 (the 4c arith->append change has not landed), so this also
    // proves the arith path stays safe on a renumbered store: it pads holes and
    // appends past the dense survivors.
    let shared = graph_with_a_deletion();
    let next_node_before = shared.read().meta.next_node_id;

    let compacted = compact_core(&shared.read()).unwrap();
    let republished = SharedGraph::from_graph(compacted.graph);

    let new_id = {
        let mut txn = republished.begin_write();
        let id = {
            let mut m = txn.mutator();
            m.create_node(
                LabelSet::single(intern("cmp.a").unwrap()),
                PropertyMap::new(),
            )
            .unwrap()
        };
        txn.commit().unwrap();
        id
    };

    let g = republished.read();
    assert_eq!(
        new_id,
        NodeId::new(next_node_before),
        "the next create must take the preserved high-water mark, not a reclaimed id"
    );
    assert!(g.is_node_alive(new_id));
    // Survivors untouched by the new allocation.
    assert!(g.is_node_alive(NodeId::new(1)));
    assert!(g.is_node_alive(NodeId::new(3)));
    assert!(g.is_node_alive(NodeId::new(4)));
    // The reclaimed deleted id is not resurrected by the new create.
    assert!(g.row_for_node_id(NodeId::new(2)).is_none());
}
