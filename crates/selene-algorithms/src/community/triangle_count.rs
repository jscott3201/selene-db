//! Per-node triangle count over the undirected view of a projection.
//!
//! Algorithm (donor `aether-db-algorithms/src/community.rs:208-248`):
//! 1. Build sorted+deduped undirected adjacency per node (out ∪ in, collapsed
//!    via `sort_unstable() + dedup()` per spec 16 §E25 / §E29 — parallel
//!    edges collapse to a single neighbor; self-loops do NOT form triangles
//!    so are filtered out at adjacency-build time).
//! 2. For each node u, for each pair `(v, w)` of u's neighbors with `v < w`,
//!    check if edge `(v, w)` exists via binary search on `adj[v]`. Each
//!    triangle contributes 1 count to each of its 3 vertices.
//!
//! Complexity: `O(V · d²)` worst case where d is the max undirected degree —
//! accepted per spec 16 §J Q12. State arrays are sized by live count via
//! `RowIndex` (§E26).

use selene_core::NodeId;

use crate::projection::GraphProjection;
use crate::structural::RowIndex;

/// Count triangles per node in the projection's undirected view.
///
/// Returns `(NodeId, count)` pairs sorted **DESC by count** with **NodeId
/// ASC** tie-break per spec 16 §E27. Empty projection → `vec![]`. Total
/// triangles equals `Σ counts / 3`; callers compute the sum themselves.
///
/// A triangle is 3 distinct mutually-connected nodes in the undirected view
/// (§E29). Self-loops do NOT form triangles; parallel edges collapse to a
/// single edge in the binary-search adjacency.
#[must_use]
pub fn triangle_count(proj: &GraphProjection) -> Vec<(NodeId, usize)> {
    let idx = RowIndex::new(proj);
    if idx.is_empty() {
        return Vec::new();
    }
    let n = idx.len();

    // Build sorted+deduped undirected adjacency per dense index. Self-loops
    // are filtered (a triangle requires 3 distinct vertices per §E29).
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); n];
    for d in 0..n as u32 {
        let node = idx.node_id_of(d);
        let neighbors = &mut adj[d as usize];
        for nb in proj.out_neighbors(node) {
            if let Some(nd) = idx.dense_of(node_sparse_row(nb.node_id))
                && nd != d
            {
                neighbors.push(nd);
            }
        }
        for nb in proj.in_neighbors(node) {
            if let Some(nd) = idx.dense_of(node_sparse_row(nb.node_id))
                && nd != d
            {
                neighbors.push(nd);
            }
        }
        neighbors.sort_unstable();
        neighbors.dedup();
    }

    let mut counts: Vec<usize> = vec![0; n];
    for u in 0..n {
        let u_neighbors = &adj[u];
        for i in 0..u_neighbors.len() {
            // Pair walk: pick `v < w` (already sorted), check `adj[v]` for w.
            // The `binary_search` gives O(log d) per pair → O(d² log d) per
            // vertex worst case. Acceptable per §J Q12.
            for j in (i + 1)..u_neighbors.len() {
                let v = u_neighbors[i];
                let w = u_neighbors[j];
                if adj[v as usize].binary_search(&w).is_ok() {
                    counts[u] += 1;
                }
            }
        }
    }

    let mut result: Vec<(NodeId, usize)> = (0..n as u32)
        .map(|d| (idx.node_id_of(d), counts[d as usize]))
        .collect();
    // §E27: DESC by count with NodeId ASC tie-break. Explicit comparator per
    // `feedback_dijkstra_tie_break_needs_both_rules`.
    result.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.get().cmp(&b.0.get())));
    result
}

#[inline]
fn node_sparse_row(nid: NodeId) -> u32 {
    (nid.get() - 1) as u32
}
