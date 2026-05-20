//! Single-source shortest path (SSSP) via Dijkstra without early-exit.
//!
//! Returns distances from `source` to every reachable node. The source itself
//! appears with `0.0` per spec 16 §E17. Self-loops at the source do not affect
//! its distance (Dijkstra invariant — distance is monotonic non-decreasing
//! from the seed).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use selene_core::{CancellationChecker, NodeId};

use crate::error::{check_algorithm, check_algorithm_stride};
use crate::pathfinding::error::PathfindingError;
use crate::projection::GraphProjection;
use crate::structural::RowIndex;

/// Internal heap entry. Same shape as `dijkstra::DijkstraEntry` but kept
/// separate to avoid cross-module visibility leaks (sssp::SsspEntry is
/// implementation detail of this file).
#[derive(Debug, Clone, Copy)]
struct SsspEntry {
    dense: u32,
    cost: f64,
}

impl PartialEq for SsspEntry {
    fn eq(&self, other: &Self) -> bool {
        self.cost.total_cmp(&other.cost) == Ordering::Equal && self.dense == other.dense
    }
}
impl Eq for SsspEntry {}
impl PartialOrd for SsspEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for SsspEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap on cost; secondary tie-break by smaller dense (§E16).
        match other.cost.total_cmp(&self.cost) {
            Ordering::Equal => other.dense.cmp(&self.dense),
            ord => ord,
        }
    }
}

/// Single-source shortest path. Returns `(NodeId, f64)` pairs sorted ASC by
/// `NodeId`, including `(source, 0.0)`, excluding unreachable nodes.
///
/// Returns `Ok(vec![])` when `source` is not in the projection. Returns
/// [`PathfindingError`] on a traversed edge with negative or NaN weight per
/// spec 16 §E15.
pub fn sssp(
    proj: &GraphProjection,
    source: NodeId,
) -> Result<Vec<(NodeId, f64)>, PathfindingError> {
    sssp_with_checker(proj, source, CancellationChecker::disabled())
}

/// Single-source shortest path with cooperative cancellation checkpoints.
pub fn sssp_with_checker(
    proj: &GraphProjection,
    source: NodeId,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, f64)>, PathfindingError> {
    check_algorithm(checker)?;
    if !proj.contains(source) {
        return Ok(Vec::new());
    }

    let idx = RowIndex::new(proj);
    if idx.is_empty() {
        return Ok(Vec::new());
    }
    let source_dense = idx
        .dense_of(node_sparse_row(source))
        .expect("source is in projection");

    let n = idx.len();
    let mut dist: Vec<f64> = vec![f64::INFINITY; n];
    let mut heap: BinaryHeap<SsspEntry> = BinaryHeap::new();

    dist[source_dense as usize] = 0.0;
    heap.push(SsspEntry {
        dense: source_dense,
        cost: 0.0,
    });

    let mut rows_since_check = 0usize;
    while let Some(SsspEntry { dense, cost }) = heap.pop() {
        check_algorithm_stride(checker, &mut rows_since_check)?;
        if cost > dist[dense as usize] {
            continue;
        }

        let source_node = idx.node_id_of(dense);
        for nb in proj.out_neighbors(source_node) {
            let Some(next_dense) = idx.dense_of(node_sparse_row(nb.node_id)) else {
                continue;
            };
            if nb.weight.is_nan() {
                return Err(PathfindingError::NaNWeight {
                    source_node,
                    target_node: nb.node_id,
                });
            }
            if nb.weight < 0.0 {
                return Err(PathfindingError::NegativeWeight {
                    source_node,
                    target_node: nb.node_id,
                    weight: nb.weight,
                });
            }

            let new_cost = cost + nb.weight;
            if new_cost < dist[next_dense as usize] {
                dist[next_dense as usize] = new_cost;
                heap.push(SsspEntry {
                    dense: next_dense,
                    cost: new_cost,
                });
            }
        }
    }

    // Materialize result in ASC-by-NodeId order. Dense indices are already
    // ASC-by-NodeId per §E03, so a single forward sweep suffices.
    let mut result: Vec<(NodeId, f64)> = Vec::with_capacity(n);
    for d in 0..n as u32 {
        let d_idx = d as usize;
        if dist[d_idx].is_finite() {
            result.push((idx.node_id_of(d), dist[d_idx]));
        }
    }
    Ok(result)
}

/// Map a `NodeId` (1-based per selene-graph) to its sparse row index (0-based).
#[inline]
fn node_sparse_row(nid: NodeId) -> u32 {
    (nid.get() - 1) as u32
}
