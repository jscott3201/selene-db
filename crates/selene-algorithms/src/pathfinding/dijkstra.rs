//! Dijkstra's single-source single-target shortest path.
//!
//! Uses `std::collections::BinaryHeap` with `f64::total_cmp` for NaN-soundness
//! in the `Ord` impl. Heap entries carry **dense indices** (per `RowIndex`),
//! NOT sparse rows — spec 16 §E14 dense-row sizing. Tie-breaks on equal cost
//! resolve by smallest dense index (= smallest NodeId per §E03 ASC sort), per
//! §E16.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use selene_core::{CancellationChecker, NodeId};

use crate::error::{check_algorithm, check_algorithm_stride};
use crate::pathfinding::error::PathfindingError;
use crate::projection::GraphProjection;
use crate::structural::RowIndex;

/// A shortest-path result: sequence of visited `NodeId`s plus total cost.
///
/// This is an **output** value type. Fields added later land via a future
/// builder/accessor pattern rather than via `#[non_exhaustive]`: as an output
/// produced inside this crate, `#[non_exhaustive]` would only bar external
/// construction and would not protect the in-crate literal construction sites,
/// so it carries the same "literal now, evolve via API later" stance as
/// `ProjectionConfig`.
#[derive(Debug, Clone, PartialEq)]
pub struct PathResult {
    /// Nodes along the path in traversal order — source first, target last.
    /// For `from == to`, this is the singleton `[from]` (zero-step path; the
    /// source is the only visit).
    pub nodes: Vec<NodeId>,
    /// Sum of edge weights along the path. `0.0` for the zero-step path.
    pub cost: f64,
}

/// Min-heap entry. Ord is reversed (smaller cost = higher priority); secondary
/// tie-break favors smaller `dense` index for §E16 determinism.
#[derive(Debug, Clone, Copy)]
struct DijkstraEntry {
    dense: u32,
    cost: f64,
}

impl PartialEq for DijkstraEntry {
    fn eq(&self, other: &Self) -> bool {
        self.cost.total_cmp(&other.cost) == Ordering::Equal && self.dense == other.dense
    }
}

impl Eq for DijkstraEntry {}

impl PartialOrd for DijkstraEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DijkstraEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse for min-heap on cost; secondary tie-break: smaller dense
        // index wins (still reversed because BinaryHeap is max-heap). Equal
        // cost + equal dense = Equal (impossible in practice but well-defined).
        match other.cost.total_cmp(&self.cost) {
            Ordering::Equal => other.dense.cmp(&self.dense),
            ord => ord,
        }
    }
}

/// Dijkstra's shortest path from `from` to `to`.
///
/// Returns `Ok(Some(PathResult))` when a path exists, `Ok(None)` when `from`
/// or `to` is not in the projection or no path exists. Returns
/// [`PathfindingError`] on a traversed edge with negative or NaN weight per
/// spec 16 §E15.
///
/// For `from == to` (and both in the projection), returns
/// `Ok(Some(PathResult { nodes: vec![from], cost: 0.0 }))` per §O.5.
pub fn dijkstra(
    proj: &GraphProjection,
    from: NodeId,
    to: NodeId,
) -> Result<Option<PathResult>, PathfindingError> {
    dijkstra_with_checker(proj, from, to, CancellationChecker::disabled())
}

/// Dijkstra's shortest path with cooperative cancellation checkpoints.
pub fn dijkstra_with_checker(
    proj: &GraphProjection,
    from: NodeId,
    to: NodeId,
    checker: CancellationChecker<'_>,
) -> Result<Option<PathResult>, PathfindingError> {
    check_algorithm(checker)?;
    if !proj.contains(from) || !proj.contains(to) {
        return Ok(None);
    }
    if from == to {
        return Ok(Some(PathResult {
            nodes: vec![from],
            cost: 0.0,
        }));
    }

    let idx = proj.row_index();
    let from_dense = idx
        .dense_of_node(from)
        .expect("from is in projection: contains() checked");
    let to_dense = idx
        .dense_of_node(to)
        .expect("to is in projection: contains() checked");

    // Why: §E14 — state sized by live count, not max_row + 1.
    let n = idx.len();
    let mut dist: Vec<f64> = vec![f64::INFINITY; n];
    let mut prev: Vec<u32> = vec![u32::MAX; n];
    let mut heap: BinaryHeap<DijkstraEntry> = BinaryHeap::new();

    dist[from_dense as usize] = 0.0;
    heap.push(DijkstraEntry {
        dense: from_dense,
        cost: 0.0,
    });

    let mut rows_since_check = 0usize;
    while let Some(DijkstraEntry { dense, cost }) = heap.pop() {
        check_algorithm_stride(checker, &mut rows_since_check)?;
        // Lazy stale-entry pruning: stale heap copies are skipped here.
        if cost > dist[dense as usize] {
            continue;
        }
        // Early-exit after the target is settled: if pop cost exceeds
        // dist[to_dense], no further relaxation can produce an equal- or
        // smaller-cost path to to_dense (pop cost is monotonic, edge weights
        // are non-negative). Returning earlier than this can lock in the wrong
        // predecessor under the Spec 16 §E16 tie-break.
        if dist[to_dense as usize].is_finite() && cost > dist[to_dense as usize] {
            break;
        }

        for nb in proj.out_neighbors_dense(dense) {
            let next_dense = nb.dense;
            // Spec 16 §E15 lazy weight validation BEFORE heap insertion.
            if nb.weight.is_nan() {
                let source_node = idx.node_id_of(dense);
                return Err(PathfindingError::NaNWeight {
                    source_node,
                    target_node: nb.node_id,
                });
            }
            if nb.weight < 0.0 {
                let source_node = idx.node_id_of(dense);
                return Err(PathfindingError::NegativeWeight {
                    source_node,
                    target_node: nb.node_id,
                    weight: nb.weight,
                });
            }

            let new_cost = cost + nb.weight;
            if new_cost < dist[next_dense as usize] {
                // Strict improvement: new shortest path found.
                dist[next_dense as usize] = new_cost;
                prev[next_dense as usize] = dense;
                heap.push(DijkstraEntry {
                    dense: next_dense,
                    cost: new_cost,
                });
            } else if new_cost == dist[next_dense as usize]
                && prev[next_dense as usize] != u32::MAX
                && dense < prev[next_dense as usize]
            {
                // Equal-cost tie-break per Spec 16 §E16: when an alternative
                // path reaches `next` at the same cost but via a smaller
                // predecessor dense index (= smaller NodeId per §E03 ASC
                // ordering), rewrite `prev` to that predecessor.
                // No heap push: cost didn't decrease, so no new relaxations
                // are needed from `next` (its dist is unchanged).
                prev[next_dense as usize] = dense;
            }
        }
    }

    if dist[to_dense as usize].is_finite() {
        let total = dist[to_dense as usize];
        Ok(Some(reconstruct(from_dense, to_dense, &prev, idx, total)))
    } else {
        Ok(None)
    }
}

/// Walk `prev[]` backwards from `to_dense` to `from_dense`, returning the
/// reconstructed `PathResult` with `nodes` in source-first order.
fn reconstruct(
    from_dense: u32,
    to_dense: u32,
    prev: &[u32],
    idx: &RowIndex,
    cost: f64,
) -> PathResult {
    let mut path = vec![idx.node_id_of(to_dense)];
    let mut cur = to_dense;
    while cur != from_dense {
        let p = prev[cur as usize];
        debug_assert_ne!(p, u32::MAX, "prev chain broke before reaching source");
        path.push(idx.node_id_of(p));
        cur = p;
    }
    path.reverse();
    PathResult { nodes: path, cost }
}
