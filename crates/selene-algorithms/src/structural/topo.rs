//! Topological sort via Kahn's algorithm (BFS over in-degree).
//!
//! Per spec 16 §E12, tie-breaks among zero-in-degree candidates resolve ASC by
//! `NodeId` for deterministic output: the ready queue is sorted before each
//! batch flush, matching donor `aether-db-algorithms/src/structural.rs:266-329`.

// Integer-keyed hot-path maps use FxHashMap to avoid SipHash overhead.
use rustc_hash::FxHashMap as HashMap;
use selene_core::NodeId;
use thiserror::Error;

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
    let total = proj.node_count();
    if total == 0 {
        return Ok(Vec::new());
    }

    // Compute in-degree against the projection (NOT the underlying graph), so
    // edges to nodes outside the projection don't inflate the count.
    let mut in_degree: HashMap<NodeId, u32> = HashMap::default();
    in_degree.reserve(total);
    for nid in proj.iter_nodes() {
        in_degree.entry(nid).or_insert(0);
        for nb in proj.out_neighbors(nid) {
            if proj.contains(nb.node_id) {
                *in_degree.entry(nb.node_id).or_insert(0) += 1;
            }
        }
    }

    // Seed the ready set with zero-in-degree nodes, sorted ASC by NodeId for
    // deterministic tie-breaks (E12).
    let mut ready: Vec<NodeId> = in_degree
        .iter()
        .filter_map(|(&nid, &deg)| (deg == 0).then_some(nid))
        .collect();
    ready.sort_by_key(|nid| nid.get());

    let mut result: Vec<(NodeId, usize)> = Vec::with_capacity(total);
    let mut position: usize = 0;

    while !ready.is_empty() {
        // Process the current batch in deterministic order; collect newly-zero
        // nodes for the next batch, then re-sort them.
        let mut next_batch: Vec<NodeId> = Vec::new();
        for nid in ready.drain(..) {
            result.push((nid, position));
            position += 1;
            for nb in proj.out_neighbors(nid) {
                if !proj.contains(nb.node_id) {
                    continue;
                }
                if let Some(deg) = in_degree.get_mut(&nb.node_id) {
                    *deg -= 1;
                    if *deg == 0 {
                        next_batch.push(nb.node_id);
                    }
                }
            }
        }
        next_batch.sort_by_key(|nid| nid.get());
        ready = next_batch;
    }

    if result.len() < total {
        // Cycle: find any node with remaining positive in-degree.
        let hint = in_degree
            .iter()
            .find_map(|(&nid, &deg)| (deg > 0).then_some(nid));
        return Err(TopoSortError::NotADag { cycle_hint: hint });
    }

    Ok(result)
}
