//! All-pairs shortest path (APSP) via repeated SSSP.
//!
//! Each call to [`crate::pathfinding::sssp`] is fully independent (fresh
//! `dist[]` allocation per source per §O.A.3). For N nodes this is O(N · sssp)
//! = O(N · (V + E) log V) for the typical sparse case — acceptable for v1.0
//! sequential pathfinding bounded by the caller-supplied `max_nodes` gate
//! (spec 16 §E18).

use selene_core::NodeId;

use rayon::prelude::*;

use crate::parallel::{ParallelRunner, Parallelism};
use crate::pathfinding::error::PathfindingError;
use crate::pathfinding::sssp::sssp;
use crate::projection::GraphProjection;

/// Configuration for all-pairs shortest path.
#[derive(Debug, Clone, Copy)]
pub struct ApspConfig {
    /// Caller-supplied upper bound for the projection node count.
    pub max_nodes: usize,
    /// Requested parallel execution policy.
    pub parallelism: Parallelism,
}

/// All-pairs shortest path. Returns `(source, target, cost)` triples sorted
/// ASC by `(source, target)`, **excluding self-pairs and unreachable pairs**
/// per spec 16 §E18.
///
/// Returns [`PathfindingError::TooLarge`] when `proj.node_count() > max_nodes`
/// (caller-supplied limit; no in-crate default per §O.4). Returns
/// [`PathfindingError::NegativeWeight`] / [`PathfindingError::NaNWeight`] on
/// invalid edge weights. Sequential mode preserves first-error-wins traversal
/// order; parallel modes may return any concurrently encountered error.
pub fn apsp(
    proj: &GraphProjection,
    config: ApspConfig,
) -> Result<Vec<(NodeId, NodeId, f64)>, PathfindingError> {
    let n = proj.node_count();
    if n > config.max_nodes {
        return Err(PathfindingError::TooLarge {
            nodes: n,
            limit: config.max_nodes,
        });
    }

    match config.parallelism {
        Parallelism::Sequential => apsp_sequential(proj),
        Parallelism::Auto | Parallelism::Threads(_) => apsp_parallel(proj, config.parallelism),
    }
}

fn apsp_sequential(proj: &GraphProjection) -> Result<Vec<(NodeId, NodeId, f64)>, PathfindingError> {
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

fn apsp_parallel(
    proj: &GraphProjection,
    parallelism: Parallelism,
) -> Result<Vec<(NodeId, NodeId, f64)>, PathfindingError> {
    let sources: Vec<NodeId> = proj.iter_nodes().collect();
    let runner =
        ParallelRunner::new(parallelism).expect("ParallelRunner builds for valid parallelism");

    runner.install(|| {
        sources
            .into_par_iter()
            .map(|source| {
                let mut rows = Vec::new();
                for (target, cost) in sssp(proj, source)? {
                    if source != target {
                        rows.push((source, target, cost));
                    }
                }
                Ok(rows)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|nested| {
                let mut result: Vec<(NodeId, NodeId, f64)> = nested.into_iter().flatten().collect();
                result.sort_by_key(|&(s, t, _)| (s.get(), t.get()));
                result
            })
    })
}
