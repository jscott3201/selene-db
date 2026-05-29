//! Dense-row remap layer for algorithm state arrays.
//!
//! Why: a `GraphProjection`'s underlying `RoaringBitmap` can be sparse (label
//! filters, scope intersections, or long-lived graphs with tombstoned deletes
//! per D11). Sizing DFS / union-find state by `max_row + 1` ties memory to the
//! largest live `NodeId` rather than the live-node count — a 100-node
//! projection at row 10⁶ would otherwise allocate megabytes per state array
//! (review-discovered invariant).
//!
//! `RowIndex` builds a one-time mapping `sparse_row ↔ dense_index` where
//! `dense_index ∈ 0..live_count`. Algorithms then size state arrays by
//! `row_index.len()` and use dense indices internally; emit-time conversion
//! recovers the `NodeId` via `row_index.node_id_of(dense_idx)`.
//!
//! The map is constructed once when the [`GraphProjection`] is built (from the
//! finalized node bitmap) and held for the projection's lifetime. Algorithms
//! borrow it via `proj.row_index()` instead of rebuilding it per call; APSP's
//! per-source O(N) remap therefore collapses to a single amortized O(N) build.
//! Because the projection's node set is frozen at build time the cached map can
//! never drift relative to its projection.

// Integer-keyed hot-path maps use FxHashMap to avoid SipHash overhead.
use roaring::RoaringBitmap;
use rustc_hash::FxHashMap as HashMap;
use selene_core::NodeId;

#[cfg(test)]
use super::GraphProjection;

/// Bidirectional `sparse_row ↔ dense_index` map for a projection's live nodes.
///
/// Built once from the projection's node bitmap (ASC by row = ASC by NodeId per
/// spec 16 §E03), assigning dense indices `0..live_count` in iteration order.
#[derive(Debug)]
pub(crate) struct RowIndex {
    /// `sparse → dense` lookup. Returns `None` for rows not in the projection
    /// (e.g., neighbor edges that point at rows outside the projection scope).
    dense: HashMap<u32, u32>,
    /// `dense → sparse` lookup, indexed by dense index.
    sparse: Vec<u32>,
}

impl RowIndex {
    /// Build the dense remap by walking `proj.iter_nodes()`.
    ///
    /// Retained for callers that hold a fully built [`GraphProjection`]; it is
    /// equivalent to [`RowIndex::from_bitmap`] over the projection's node set.
    #[cfg(test)]
    pub(crate) fn new(proj: &GraphProjection) -> Self {
        let sparse: Vec<u32> = proj
            .iter_nodes()
            .map(|nid| (nid.get() - 1) as u32)
            .collect();
        Self::from_sparse(sparse)
    }

    /// Build the dense remap directly from a row-indexed node bitmap.
    ///
    /// Used by [`GraphProjection::build`] after the node set is finalized.
    /// `RoaringBitmap::iter` yields rows ASC, which equals `iter_nodes()` ASC
    /// (and therefore ASC by NodeId per spec 16 §E03), so the dense-index
    /// assignment is byte-identical to [`RowIndex::new`].
    pub(crate) fn from_bitmap(nodes: &RoaringBitmap) -> Self {
        Self::from_sparse(nodes.iter().collect())
    }

    fn from_sparse(sparse: Vec<u32>) -> Self {
        let dense: HashMap<u32, u32> = sparse
            .iter()
            .enumerate()
            .map(|(i, &r)| (r, i as u32))
            .collect();
        Self { dense, sparse }
    }

    /// Number of live rows in the projection.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.sparse.len()
    }

    /// Returns `true` when there are no live rows.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.sparse.is_empty()
    }

    /// `sparse_row → dense_index`, or `None` when `sparse_row` is not in the
    /// projection. Callers should treat `None` as "neighbor lives outside the
    /// projection scope; skip this edge" — matches the existing
    /// `proj.contains(...)` guard pattern used in topo's in-degree counting.
    #[inline]
    pub(crate) fn dense_of(&self, sparse_row: u32) -> Option<u32> {
        self.dense.get(&sparse_row).copied()
    }

    /// `NodeId → dense_index`, or `None` when the node is not in the projection.
    ///
    /// Convenience over [`RowIndex::dense_of`] that applies the
    /// `node_row_index` inverse (`id.get() - 1`) inline, collapsing the
    /// per-algorithm `node_sparse_row` helpers into one place.
    #[inline]
    pub(crate) fn dense_of_node(&self, node: NodeId) -> Option<u32> {
        self.dense_of((node.get() - 1) as u32)
    }

    /// `dense_index → NodeId`. Panics on out-of-range dense indices, which
    /// would indicate an algorithm bug rather than a data issue.
    #[inline]
    pub(crate) fn node_id_of(&self, dense_idx: u32) -> NodeId {
        let row = self.sparse[dense_idx as usize];
        NodeId::new(u64::from(row) + 1)
    }
}
