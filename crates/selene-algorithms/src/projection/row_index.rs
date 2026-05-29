//! Dense-row remap layer for algorithm state arrays.
//!
//! Why: a `GraphProjection`'s underlying `RoaringBitmap` can be sparse (label
//! filters, scope intersections, or long-lived graphs with tombstoned deletes
//! per D11). Sizing DFS / union-find state by `max_row + 1` ties memory to the
//! largest live row rather than the live-node count — a 100-node projection at
//! row 10⁶ would otherwise allocate megabytes per state array (review-discovered
//! invariant).
//!
//! `RowIndex` builds a one-time mapping `sparse_row ↔ dense_index` where
//! `dense_index ∈ 0..live_count`, and captures each row's external [`NodeId`]
//! from the graph's `row_to_id` column at build time (BRIEF-Item-4a). Algorithms
//! size state arrays by `row_index.len()`, use dense indices internally, and
//! recover the `NodeId` via [`RowIndex::node_id_of`] — never by `row + 1`
//! arithmetic, so the external id stays stable while a 4b compaction renumbers
//! the row.
//!
//! The map is constructed once when the [`GraphProjection`](super::GraphProjection)
//! is built (from the finalized node bitmap + the snapshot) and held for the
//! projection's lifetime. Because the projection's node set is frozen at build
//! time the cached map can never drift relative to its projection.

// Integer-keyed hot-path maps use FxHashMap to avoid SipHash overhead.
use roaring::RoaringBitmap;
use rustc_hash::FxHashMap as HashMap;
use selene_core::NodeId;
use selene_graph::{RowIndex as GraphRowIndex, SeleneGraph};

/// Bidirectional remap for a projection's live nodes: `sparse_row ↔ dense_index`
/// and `dense_index ↔ external NodeId`.
///
/// Built once from the projection's node bitmap (ASC by row per spec 16 §E03),
/// assigning dense indices `0..live_count` in iteration order.
#[derive(Debug)]
pub(crate) struct RowIndex {
    /// `sparse_row → dense_index`. Returns `None` for rows not in the projection
    /// (e.g., neighbor edges that point at rows outside the projection scope).
    dense: HashMap<u32, u32>,
    /// `dense_index → external NodeId`, captured at build time from the graph's
    /// `row_to_id` column — never synthesized as `row + 1`.
    node_ids: Vec<NodeId>,
    /// `external NodeId → dense_index`, the reverse of [`Self::node_ids`].
    node_to_dense: HashMap<NodeId, u32>,
}

impl RowIndex {
    /// Build the dense remap from a row-indexed node bitmap, capturing each
    /// row's external [`NodeId`] from `snapshot`'s `row_to_id` column.
    ///
    /// Used by [`GraphProjection::build`](super::GraphProjection::build) after
    /// the node set is finalized. `RoaringBitmap::iter` yields rows ASC; the
    /// captured `node_ids` are therefore in dense order, and (under the
    /// BRIEF-Item-4a identity mapping) ASC by NodeId per spec 16 §E03.
    pub(crate) fn from_bitmap(nodes: &RoaringBitmap, snapshot: &SeleneGraph) -> Self {
        let rows: Vec<u32> = nodes.iter().collect();
        let node_ids: Vec<NodeId> = rows
            .iter()
            .map(|&row| {
                snapshot
                    .node_id_for_row(GraphRowIndex::new(row))
                    .expect("projection row is alive and has a mapped external id")
            })
            .collect();
        let dense: HashMap<u32, u32> = rows
            .iter()
            .enumerate()
            .map(|(i, &r)| (r, i as u32))
            .collect();
        let node_to_dense: HashMap<NodeId, u32> = node_ids
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i as u32))
            .collect();
        Self {
            dense,
            node_ids,
            node_to_dense,
        }
    }

    /// Number of live rows in the projection.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.node_ids.len()
    }

    /// Returns `true` when there are no live rows.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.node_ids.is_empty()
    }

    /// `sparse_row → dense_index`, or `None` when `sparse_row` is not in the
    /// projection. Callers should treat `None` as "neighbor lives outside the
    /// projection scope; skip this edge".
    #[inline]
    pub(crate) fn dense_of(&self, sparse_row: u32) -> Option<u32> {
        self.dense.get(&sparse_row).copied()
    }

    /// `external NodeId → dense_index`, or `None` when the node is not in the
    /// projection. The map-backed replacement for the old `id - 1` arithmetic.
    #[inline]
    pub(crate) fn dense_of_node(&self, node: NodeId) -> Option<u32> {
        self.node_to_dense.get(&node).copied()
    }

    /// `dense_index → external NodeId`. Panics on out-of-range dense indices,
    /// which would indicate an algorithm bug rather than a data issue.
    #[inline]
    pub(crate) fn node_id_of(&self, dense_idx: u32) -> NodeId {
        self.node_ids[dense_idx as usize]
    }

    /// Iterate the projection's external `NodeId`s in dense (ASC-by-row) order.
    #[inline]
    pub(crate) fn iter_node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.node_ids.iter().copied()
    }
}
