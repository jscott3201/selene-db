//! Dense-row remap layer for structural-algorithm state arrays.
//!
//! Why: a `GraphProjection`'s underlying `RoaringBitmap` can be sparse (label
//! filters, scope intersections, or long-lived graphs with tombstoned deletes
//! per D11). Sizing DFS / union-find state by `max_row + 1` ties memory to the
//! largest live `NodeId` rather than the live-node count — a 100-node
//! projection at row 10⁶ would otherwise allocate megabytes per state array
//! (Codex P1 review on BRIEF-52 PR #58).
//!
//! `RowIndex` builds a one-time mapping `sparse_row ↔ dense_index` where
//! `dense_index ∈ 0..live_count`. Algorithms then size state arrays by
//! `row_index.len()` and use dense indices internally; emit-time conversion
//! recovers the `NodeId` via `row_index.node_id_of(dense_idx)`.

// Integer-keyed hot-path maps use FxHashMap to avoid SipHash overhead.
use rustc_hash::FxHashMap as HashMap;
use selene_core::NodeId;

use crate::projection::GraphProjection;

/// Bidirectional `sparse_row ↔ dense_index` map for a projection's live nodes.
///
/// Built once per algorithm invocation by walking `proj.iter_nodes()` (ASC by
/// NodeId per spec 16 §E03), assigning dense indices `0..live_count` in
/// iteration order.
pub(crate) struct RowIndex {
    /// `sparse → dense` lookup. Returns `None` for rows not in the projection
    /// (e.g., neighbor edges that point at rows outside the projection scope).
    dense: HashMap<u32, u32>,
    /// `dense → sparse` lookup, indexed by dense index.
    sparse: Vec<u32>,
}

impl RowIndex {
    /// Build the dense remap by walking `proj.iter_nodes()`.
    pub(crate) fn new(proj: &GraphProjection) -> Self {
        let sparse: Vec<u32> = proj
            .iter_nodes()
            .map(|nid| (nid.get() - 1) as u32)
            .collect();
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

    /// `dense_index → NodeId`. Panics on out-of-range dense indices, which
    /// would indicate an algorithm bug rather than a data issue.
    #[inline]
    pub(crate) fn node_id_of(&self, dense_idx: u32) -> NodeId {
        let row = self.sparse[dense_idx as usize];
        NodeId::new(u64::from(row) + 1)
    }
}
