//! Topological sort via Kahn's algorithm (BFS over in-degree).
//!
//! Per spec 16 §E12, tie-breaks among zero-in-degree candidates resolve ASC by
//! `NodeId` for deterministic output: the ready queue is sorted before each
//! batch flush, matching donor `aether-db-algorithms/src/structural.rs:266-329`.

use selene_core::{CancellationChecker, NodeId};
use thiserror::Error;

use crate::error::{AlgorithmAborted, check_algorithm, check_algorithm_stride};
use crate::projection::GraphProjection;

/// Error returned by [`topological_sort`] when the projection is not a DAG.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TopoSortError {
    /// The projection contains at least one directed cycle. `cycle_hint`
    /// points at one node that remained with positive in-degree when the
    /// Kahn-style sweep stalled, indicating it sits on (or is reachable from)
    /// a cycle. `None` only when the projection is empty — which is otherwise
    /// reported as `Ok(vec![])` per spec 16 §E09; this variant is reserved for
    /// degenerate states.
    #[error("projection contains a directed cycle (hint: {cycle_hint:?})")]
    NotADag {
        /// A node observed on (or reachable from) a cycle.
        cycle_hint: Option<NodeId>,
    },

    /// Algorithm observed cooperative cancellation or deadline expiry.
    #[error(transparent)]
    Aborted {
        /// Cancellation cause.
        #[from]
        source: AlgorithmAborted,
    },
}

/// Topologically sort the projection in directed-acyclic order.
///
/// Returns `(NodeId, topo_position)` pairs ordered by `topo_position` ASC.
/// `topo_position` is a 0-based monotonic counter; tie-breaks (multiple
/// zero-in-degree candidates) resolve ASC by `NodeId`.
///
/// Returns [`TopoSortError::NotADag`] when a directed cycle is present. An
/// empty projection returns `Ok(vec![])` per spec 16 §E09.
pub fn topological_sort(proj: &GraphProjection) -> Result<Vec<(NodeId, usize)>, TopoSortError> {
    topological_sort_with_checker(proj, CancellationChecker::disabled())
}

/// Topologically sort the projection with cooperative cancellation checkpoints.
pub fn topological_sort_with_checker(
    proj: &GraphProjection,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, usize)>, TopoSortError> {
    check_algorithm(checker)?;
    let total = proj.node_count();
    if total == 0 {
        return Ok(Vec::new());
    }
    let idx = proj.row_index();

    // Compute in-degree against the projection (NOT the underlying graph), so
    // edges to nodes outside the projection don't inflate the count. The CSR
    // already stores only projected neighbors and caches each neighbor's dense
    // row, so this hot path can stay in contiguous arrays.
    let mut in_degree = vec![0u32; total];
    let mut rows_since_check = 0usize;
    for dense in 0..total as u32 {
        check_algorithm_stride(checker, &mut rows_since_check)?;
        for nb in proj.out_neighbors_dense(dense) {
            in_degree[nb.dense as usize] += 1;
        }
    }

    // Seed the ready set with zero-in-degree nodes, sorted ASC by NodeId for
    // deterministic tie-breaks (E12). RowIndex dense order is ASC by NodeId.
    let mut ready: Vec<u32> = in_degree
        .iter()
        .enumerate()
        .filter_map(|(dense, &deg)| (deg == 0).then_some(dense as u32))
        .collect();

    let mut result: Vec<(NodeId, usize)> = Vec::with_capacity(total);
    let mut position: usize = 0;

    while !ready.is_empty() {
        check_algorithm(checker)?;
        // Process the current batch in deterministic order; collect newly-zero
        // nodes for the next batch, then re-sort them.
        let mut next_batch: Vec<u32> = Vec::new();
        rows_since_check = 0;
        for dense in ready.drain(..) {
            check_algorithm_stride(checker, &mut rows_since_check)?;
            result.push((idx.node_id_of(dense), position));
            position += 1;
            for nb in proj.out_neighbors_dense(dense) {
                let deg = &mut in_degree[nb.dense as usize];
                *deg -= 1;
                if *deg == 0 {
                    next_batch.push(nb.dense);
                }
            }
        }
        next_batch.sort_unstable();
        ready = next_batch;
    }

    if result.len() < total {
        // Cycle: find any node with remaining positive in-degree.
        let hint = in_degree
            .iter()
            .enumerate()
            .find_map(|(dense, &deg)| (deg > 0).then_some(idx.node_id_of(dense as u32)));
        return Err(TopoSortError::NotADag { cycle_hint: hint });
    }

    Ok(result)
}
