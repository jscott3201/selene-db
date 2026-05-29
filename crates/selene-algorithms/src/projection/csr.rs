//! Compressed-sparse-row adjacency storage for `GraphProjection`.
//!
//! CSR is keyed by **dense index**, not sparse row. Every value admitted to a
//! projection's `nodes` bitmap is a sparse *row index* sourced from
//! `snapshot.live_nodes()` or `snapshot.nodes_with_label()`; the projection's
//! cached [`RowIndex`] maps each sparse row to a dense index `0..live_count`.
//! Offsets are therefore sized by the live-node count (not `node_store.len()`),
//! so a 100-node projection over a 1M-row store allocates ~100 offsets, not ~1M
//! (review-discovered memory invariant). Converting a neighbor's `NodeId` back
//! to a sparse row (`id.get() - 1` per the `node_row_index` invariant in
//! `selene_graph::store`) and then to its dense index is always valid for
//! neighbors inside the projection; mismatched bitmap state surfaces as an
//! `expect()` panic with a diagnostic message.
//!
//! CSR neighbor/offset count bound: `offsets: Vec<u32>` and the per-bucket
//! offsets are u32-indexed, so total projected neighbor count is bounded by
//! `u32::MAX` (~4.3 B), aligning with selene-graph's row addressing. Production
//! graphs approaching this bound are out of scope for v1.0's in-memory algorithm
//! surface — distributed algorithms handle them.

use roaring::RoaringBitmap;
use selene_core::{EdgeId, IStr, NodeId, Value};
use selene_graph::{SeleneGraph, store::node_row_index};

use super::RowIndex;

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
/// `offsets[dense]..offsets[dense+1]` is the neighbor slice for the node at that
/// dense index (assigned by the projection's [`RowIndex`]). `offsets` is sized
/// `live_count + 1`; the trailing slot is the running total sentinel.
#[derive(Debug)]
pub(crate) struct ProjCsr {
    offsets: Vec<u32>,
    neighbors: Vec<ProjNeighbor>,
}

impl ProjCsr {
    /// Neighbors of the node at dense index `dense`. Empty slice when the index
    /// is out of range or when the node has no qualifying neighbors.
    pub(crate) fn neighbors_of_dense(&self, dense: u32) -> &[ProjNeighbor] {
        let dense = dense as usize;
        if dense + 1 >= self.offsets.len() {
            return &[];
        }
        let start = self.offsets[dense] as usize;
        let end = self.offsets[dense + 1] as usize;
        &self.neighbors[start..end]
    }

    /// Total neighbor count across all rows.
    pub(crate) fn total_neighbors(&self) -> usize {
        self.neighbors.len()
    }

    /// Length of the offsets vector (`live_count + 1`). Test-only accessor that
    /// proves dense (not sparse) offset sizing.
    #[cfg(test)]
    pub(crate) fn offsets_len(&self) -> usize {
        self.offsets.len()
    }
}

/// Build the outgoing-direction CSR for a projection.
pub(crate) fn build_csr_out(
    snapshot: &SeleneGraph,
    nodes: &RoaringBitmap,
    row_index: &RowIndex,
    edge_labels: &[IStr],
    weight_property: Option<&IStr>,
) -> ProjCsr {
    build_csr(
        snapshot,
        nodes,
        row_index,
        edge_labels,
        weight_property,
        Direction::Out,
    )
}

/// Build the incoming-direction CSR for a projection.
pub(crate) fn build_csr_in(
    snapshot: &SeleneGraph,
    nodes: &RoaringBitmap,
    row_index: &RowIndex,
    edge_labels: &[IStr],
    weight_property: Option<&IStr>,
) -> ProjCsr {
    build_csr(
        snapshot,
        nodes,
        row_index,
        edge_labels,
        weight_property,
        Direction::In,
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Out,
    In,
}

fn build_csr(
    snapshot: &SeleneGraph,
    nodes: &RoaringBitmap,
    row_index: &RowIndex,
    edge_labels: &[IStr],
    weight_property: Option<&IStr>,
    direction: Direction,
) -> ProjCsr {
    // Invariant: `row_index` is built from exactly this `nodes` bitmap (see
    // `GraphProjection::build`), so every row enumerated below has a dense
    // index. Size offsets to dense (live-node) count + 1 so
    // offsets[dense]..offsets[dense+1] is always a valid range; the +1 slot
    // holds the running total after the prefix sum (sentinel).
    debug_assert_eq!(
        nodes.len() as usize,
        row_index.len(),
        "CSR row_index must be built from the same nodes bitmap"
    );
    let dense_n = row_index.len();
    let mut offsets = vec![0u32; dense_n + 1];

    // selene-graph adjacency entries inline label + neighbor + edge_id; no
    // per-edge id → data lookup is needed during the filter pass (cf. donor
    // aether-db-algorithms projection.rs:248 which calls graph.get_edge per
    // edge id). Only weight extraction performs an additional lookup, gated
    // by `weight_property.is_some()`.

    // Pass 1: count qualifying neighbors per dense index.
    for row_u32 in nodes {
        let nid = row_to_node_id(row_u32);
        let dense = row_index
            .dense_of(row_u32)
            .expect("projection row has a dense index") as usize;
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
            offsets[dense] += 1;
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

    // Pass 2: fill the neighbor array using a per-dense-index write cursor.
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
        let dense = row_index
            .dense_of(row_u32)
            .expect("projection row has a dense index") as usize;
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
            let pos = cursor[dense] as usize;
            neighbors[pos] = ProjNeighbor {
                edge_id: adj.edge_id,
                node_id: adj.neighbor,
                weight,
            };
            cursor[dense] += 1;
        }
    }

    // Per spec 16 §E03: sort each row's bucket by node_id ASC. selene-graph's
    // AdjacencyEntry is sorted by (label, neighbor, edge_id); after filtering
    // by edge_labels the per-row order is not pure ASC-by-neighbor, so we
    // re-sort explicitly to guarantee deterministic algorithm tie-breaks and
    // snapshot-harness goldens.
    for d in 0..dense_n {
        let start = offsets[d] as usize;
        let end = offsets[d + 1] as usize;
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
