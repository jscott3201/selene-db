//! All-pairs shortest path (APSP) via repeated SSSP.
//!
//! Each call to [`crate::pathfinding::sssp`] is fully independent (fresh
//! `dist[]` allocation per source per §O.A.3). For N nodes this is O(N · sssp)
//! = O(N · (V + E) log V) for the typical sparse case — acceptable for v1.0
//! sequential pathfinding bounded by the caller-supplied `max_nodes` gate
//! (spec 16 §E18).

use selene_core::NodeId;

use crate::pathfinding::error::PathfindingError;
use crate::pathfinding::sssp::sssp;
use crate::projection::GraphProjection;

/// All-pairs shortest path. Returns `(source, target, cost)` triples sorted
/// ASC by `(source, target)`, **excluding self-pairs and unreachable pairs**
/// per spec 16 §E18.
///
/// Returns [`PathfindingError::TooLarge`] when `proj.node_count() > max_nodes`
/// (caller-supplied limit; no in-crate default per §O.4). Returns
/// [`PathfindingError::NegativeWeight`] / [`PathfindingError::NaNWeight`] on
/// the first offending edge encountered across any source's traversal.
pub fn apsp(
    proj: &GraphProjection,
    max_nodes: usize,
) -> Result<Vec<(NodeId, NodeId, f64)>, PathfindingError> {
    let n = proj.node_count();
    if n > max_nodes {
        return Err(PathfindingError::TooLarge {
            nodes: n,
            limit: max_nodes,
        });
    }

    // Why: `iter_nodes()` returns ASC by NodeId per spec 16 §E03, so
    // result.push() order is already ASC by source; we only need to ensure
    // each sssp call returns ASC by target (which it does per §E17), then
    // the final `result` is already sorted. A defensive sort at the end
    // tolerates any future SSSP reordering without breaking §E18.
    //
    // Why no `Vec::with_capacity(n * n)` preallocation (PR #59 Codex P2):
    // sparse graphs have far fewer than n² reachable pairs, and n=10_000 (a
    // plausible `max_nodes`) would reserve 2.4 GB up front even if the
    // result has only thousands of pairs. Let the Vec grow naturally; the
    // amortized cost is bounded by the actual result size.
    let mut result: Vec<(NodeId, NodeId, f64)> = Vec::new();
    for source in proj.iter_nodes() {
        for (target, cost) in sssp(proj, source)? {
            if source != target {
                result.push((source, target, cost));
            }
        }
    }
    result.sort_by_key(|&(s, t, _)| (s.get(), t.get()));
    Ok(result)
}
