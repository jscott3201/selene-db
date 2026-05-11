//! Compressed-sparse-row adjacency storage for `GraphProjection`.
//!
//! CSR row-indexing safety: every value admitted to a projection's `nodes`
//! bitmap is a *row index* sourced from `snapshot.live_nodes()` or
//! `snapshot.nodes_with_label()`, both of which return row indices in
//! `[0, node_store.len())`. Converting back to a `NodeId` (`row + 1` per the
//! `node_row_index` invariant in `selene_graph::store`) is therefore always
//! valid; mismatched bitmap state surfaces as an `expect()` panic with a
//! diagnostic message.
//!
//! CSR neighbor/offset count bound: `offsets: Vec<u32>` and the per-bucket
//! offsets are u32-indexed, so total projected neighbor count is bounded by
//! `u32::MAX` (~4.3 B), aligning with selene-graph's row addressing. Production
//! graphs approaching this bound are out of scope for v1.0's in-memory algorithm
//! surface — distributed algorithms handle them.

use roaring::RoaringBitmap;
use selene_core::{EdgeId, IStr, NodeId, Value};
use selene_graph::{SeleneGraph, store::node_row_index};

/// One neighbor entry in a projection's CSR adjacency.
#[derive(Debug, Clone, Copy)]
pub struct ProjNeighbor {
    /// Edge ID underlying this projected neighbor link.
    pub edge_id: EdgeId,
    /// Neighbor node ID (target for outgoing CSR, source for incoming CSR).
    pub node_id: NodeId,
    /// Edge weight projected from the source graph. Defaults to `1.0` when
    /// `weight_property` is `None` or when the property is missing /
    /// non-numeric / `Value::Null` per spec 16 §3 E04 (matches the donor's
    /// permissive behavior).
    pub weight: f64,
}

/// CSR adjacency for one direction within a projection.
///
/// `offsets[row]..offsets[row+1]` is the neighbor slice for the node at that
/// row. Rows for nodes not in the projection's `nodes` bitmap have an empty
/// range.
#[derive(Debug)]
pub(crate) struct ProjCsr {
    offsets: Vec<u32>,
    neighbors: Vec<ProjNeighbor>,
}

impl ProjCsr {
    /// Neighbors of the node at `row`. Empty slice when the row is out of range
    /// or when the node has no qualifying neighbors in this projection.
    pub(crate) fn neighbors_of_row(&self, row: u32) -> &[ProjNeighbor] {
        let row = row as usize;
        if row + 1 >= self.offsets.len() {
            return &[];
        }
        let start = self.offsets[row] as usize;
        let end = self.offsets[row + 1] as usize;
        &self.neighbors[start..end]
    }

    /// Total neighbor count across all rows.
    pub(crate) fn total_neighbors(&self) -> usize {
        self.neighbors.len()
    }
}

/// Build the outgoing-direction CSR for a projection.
pub(crate) fn build_csr_out(
    snapshot: &SeleneGraph,
    nodes: &RoaringBitmap,
    edge_labels: &[IStr],
    weight_property: Option<&IStr>,
) -> ProjCsr {
    build_csr(
        snapshot,
        nodes,
        edge_labels,
        weight_property,
        Direction::Out,
    )
}

/// Build the incoming-direction CSR for a projection.
pub(crate) fn build_csr_in(
    snapshot: &SeleneGraph,
    nodes: &RoaringBitmap,
    edge_labels: &[IStr],
    weight_property: Option<&IStr>,
) -> ProjCsr {
    build_csr(snapshot, nodes, edge_labels, weight_property, Direction::In)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Out,
    In,
}

fn build_csr(
    snapshot: &SeleneGraph,
    nodes: &RoaringBitmap,
    edge_labels: &[IStr],
    weight_property: Option<&IStr>,
    direction: Direction,
) -> ProjCsr {
    // Size offsets to node_store.len() + 1 so offsets[row]..offsets[row+1] is
    // always a valid range. The +1 slot holds the running total after the
    // prefix sum (sentinel).
    let store_len = snapshot.node_store.len();
    let mut offsets = vec![0u32; store_len + 1];

    // selene-graph adjacency entries inline label + neighbor + edge_id; no
    // per-edge id → data lookup is needed during the filter pass (cf. donor
    // aether-db-algorithms projection.rs:248 which calls graph.get_edge per
    // edge id). Only weight extraction performs an additional lookup, gated
    // by `weight_property.is_some()`.

    // Pass 1: count qualifying neighbors per node row.
    for row_u32 in nodes {
        let nid = row_to_node_id(row_u32);
        let row = row_u32 as usize;
        let Some(entry) = (match direction {
            Direction::Out => snapshot.outgoing_edges(nid),
            Direction::In => snapshot.incoming_edges(nid),
        }) else {
            continue;
        };
        for adj in entry.iter() {
            if !contains_neighbor(nodes, adj.neighbor) {
                continue;
            }
            if !edge_labels.is_empty() && !edge_labels.contains(&adj.label) {
                continue;
            }
            offsets[row] += 1;
        }
    }

    // Convert per-row counts into cumulative offsets (prefix sum). The final
    // slot holds the total neighbor count.
    let mut cumulative: u32 = 0;
    for slot in offsets.iter_mut() {
        let count = *slot;
        *slot = cumulative;
        cumulative = cumulative.saturating_add(count);
    }

    // Pass 2: fill the neighbor array using a per-row write cursor.
    let total = cumulative as usize;
    let mut neighbors: Vec<ProjNeighbor> = vec![
        ProjNeighbor {
            edge_id: EdgeId::new(0),
            node_id: NodeId::new(0),
            weight: 0.0,
        };
        total
    ];
    let mut cursor = offsets.clone();

    for row_u32 in nodes {
        let nid = row_to_node_id(row_u32);
        let row = row_u32 as usize;
        let Some(entry) = (match direction {
            Direction::Out => snapshot.outgoing_edges(nid),
            Direction::In => snapshot.incoming_edges(nid),
        }) else {
            continue;
        };
        for adj in entry.iter() {
            if !contains_neighbor(nodes, adj.neighbor) {
                continue;
            }
            if !edge_labels.is_empty() && !edge_labels.contains(&adj.label) {
                continue;
            }
            let weight = extract_weight(snapshot, adj.edge_id, weight_property);
            let pos = cursor[row] as usize;
            neighbors[pos] = ProjNeighbor {
                edge_id: adj.edge_id,
                node_id: adj.neighbor,
                weight,
            };
            cursor[row] += 1;
        }
    }

    // Per spec 16 §E03: sort each row's bucket by node_id ASC. selene-graph's
    // AdjacencyEntry is sorted by (label, neighbor, edge_id); after filtering
    // by edge_labels the per-row order is not pure ASC-by-neighbor, so we
    // re-sort explicitly to guarantee deterministic algorithm tie-breaks and
    // snapshot-harness goldens.
    for row in 0..store_len {
        let start = offsets[row] as usize;
        let end = offsets[row + 1] as usize;
        if end > start {
            neighbors[start..end].sort_by_key(|n| n.node_id);
        }
    }

    ProjCsr { offsets, neighbors }
}

/// Reconstruct a `NodeId` from a row index. selene-graph's `node_row_index`
/// invariant is `id.get() == row + 1` for any alive node, so the inverse is
/// straightforward.
fn row_to_node_id(row: u32) -> NodeId {
    NodeId::new(u64::from(row) + 1)
}

/// Membership test for a `NodeId` against a row-indexed bitmap.
fn contains_neighbor(nodes: &RoaringBitmap, neighbor: NodeId) -> bool {
    match node_row_index(neighbor) {
        Some(row) => nodes.contains(row),
        None => false,
    }
}

/// Extract the edge weight per spec 16 §E04 (permissive: missing / non-numeric
/// / null → `1.0`).
fn extract_weight(snapshot: &SeleneGraph, edge_id: EdgeId, weight_property: Option<&IStr>) -> f64 {
    let Some(prop) = weight_property else {
        return 1.0;
    };
    let Some(properties) = snapshot.edge_properties(edge_id) else {
        return 1.0;
    };
    match properties.get(prop) {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f64,
        Some(Value::Uint(u)) => *u as f64,
        _ => 1.0,
    }
}
