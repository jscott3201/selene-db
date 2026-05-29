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

use selene_core::{CancellationChecker, NodeId};

use rayon::prelude::*;

use crate::error::{AlgorithmAborted, check_algorithm, check_algorithm_stride};
use crate::parallel::{ParallelRunner, Parallelism};
use crate::projection::GraphProjection;
use crate::structural::RowIndex;

/// Configuration for per-node triangle count.
#[derive(Debug, Clone, Copy, Default)]
pub struct TriangleCountConfig {
    /// Requested parallel execution policy.
    pub parallelism: Parallelism,
}

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
pub fn triangle_count(proj: &GraphProjection, config: TriangleCountConfig) -> Vec<(NodeId, usize)> {
    match config.parallelism {
        Parallelism::Sequential => triangle_count_sequential(proj),
        Parallelism::Auto | Parallelism::Threads(_) => {
            triangle_count_parallel(proj, config.parallelism)
        }
    }
}

/// Count triangles per node with cooperative cancellation checkpoints.
pub fn triangle_count_with_checker(
    proj: &GraphProjection,
    config: TriangleCountConfig,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, usize)>, AlgorithmAborted> {
    if checker.is_disabled() {
        return Ok(triangle_count(proj, config));
    }

    match config.parallelism {
        Parallelism::Sequential => triangle_count_sequential_checked(proj, checker),
        Parallelism::Auto | Parallelism::Threads(_) => {
            triangle_count_parallel_checked(proj, config.parallelism, checker)
        }
    }
}

fn triangle_count_sequential(proj: &GraphProjection) -> Vec<(NodeId, usize)> {
    let adjacency = build_dense_adjacency(proj);
    if adjacency.is_empty() {
        return Vec::new();
    }

    let result = (0..adjacency.row_count())
        .map(|row| {
            (
                adjacency.node_at_row(row),
                count_triangles_at_row(row, &adjacency),
            )
        })
        .collect();
    sort_triangle_count_results(result)
}

fn triangle_count_sequential_checked(
    proj: &GraphProjection,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, usize)>, AlgorithmAborted> {
    let adjacency = build_dense_adjacency_checked(proj, checker)?;
    if adjacency.is_empty() {
        return Ok(Vec::new());
    }

    let mut rows_since_check = 0usize;
    let result = (0..adjacency.row_count())
        .map(|row| {
            check_algorithm_stride(checker, &mut rows_since_check)?;
            Ok((
                adjacency.node_at_row(row),
                count_triangles_at_row(row, &adjacency),
            ))
        })
        .collect::<Result<Vec<_>, AlgorithmAborted>>()?;
    Ok(sort_triangle_count_results(result))
}

fn triangle_count_parallel(
    proj: &GraphProjection,
    parallelism: Parallelism,
) -> Vec<(NodeId, usize)> {
    let adjacency = build_dense_adjacency(proj);
    if adjacency.is_empty() {
        return Vec::new();
    }

    let runner =
        ParallelRunner::new(parallelism).expect("ParallelRunner builds for valid parallelism");
    let result = runner.install(|| {
        (0..adjacency.row_count())
            .into_par_iter()
            .map(|row| {
                (
                    adjacency.node_at_row(row),
                    count_triangles_at_row(row, &adjacency),
                )
            })
            .collect()
    });
    sort_triangle_count_results(result)
}

fn triangle_count_parallel_checked(
    proj: &GraphProjection,
    parallelism: Parallelism,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, usize)>, AlgorithmAborted> {
    let adjacency = build_dense_adjacency_checked(proj, checker)?;
    if adjacency.is_empty() {
        return Ok(Vec::new());
    }

    let runner =
        ParallelRunner::new(parallelism).expect("ParallelRunner builds for valid parallelism");
    let result = runner.install(|| {
        (0..adjacency.row_count())
            .into_par_iter()
            .map(|row| {
                check_algorithm(checker)?;
                Ok((
                    adjacency.node_at_row(row),
                    count_triangles_at_row(row, &adjacency),
                ))
            })
            .collect::<Result<Vec<_>, AlgorithmAborted>>()
    })?;
    Ok(sort_triangle_count_results(result))
}

struct DenseAdjacency<'a> {
    idx: &'a RowIndex,
    adj: Vec<Vec<u32>>,
}

impl DenseAdjacency<'_> {
    fn is_empty(&self) -> bool {
        self.idx.is_empty()
    }

    fn row_count(&self) -> usize {
        self.idx.len()
    }

    fn node_at_row(&self, row: usize) -> NodeId {
        self.idx.node_id_of(row as u32)
    }

    fn neighbors(&self, row: usize) -> &[u32] {
        &self.adj[row]
    }
}

fn build_dense_adjacency(proj: &GraphProjection) -> DenseAdjacency<'_> {
    let idx = proj.row_index();
    let n = idx.len();

    // Build sorted+deduped undirected adjacency per dense index. Self-loops
    // are filtered (a triangle requires 3 distinct vertices per §E29).
    let mut adj = vec![Vec::new(); n];
    for d in 0..n as u32 {
        let node = idx.node_id_of(d);
        let neighbors = &mut adj[d as usize];
        for nb in proj.out_neighbors(node) {
            if let Some(nd) = idx.dense_of_node(nb.node_id)
                && nd != d
            {
                neighbors.push(nd);
            }
        }
        for nb in proj.in_neighbors(node) {
            if let Some(nd) = idx.dense_of_node(nb.node_id)
                && nd != d
            {
                neighbors.push(nd);
            }
        }
        neighbors.sort_unstable();
        neighbors.dedup();
    }
    DenseAdjacency { idx, adj }
}

fn build_dense_adjacency_checked<'a>(
    proj: &'a GraphProjection,
    checker: CancellationChecker<'_>,
) -> Result<DenseAdjacency<'a>, AlgorithmAborted> {
    check_algorithm(checker)?;
    let idx = proj.row_index();
    let n = idx.len();

    // Build sorted+deduped undirected adjacency per dense index. Self-loops
    // are filtered (a triangle requires 3 distinct vertices per §E29).
    let mut adj = vec![Vec::new(); n];
    let mut rows_since_check = 0usize;
    for d in 0..n as u32 {
        check_algorithm_stride(checker, &mut rows_since_check)?;
        let node = idx.node_id_of(d);
        let neighbors = &mut adj[d as usize];
        for nb in proj.out_neighbors(node) {
            if let Some(nd) = idx.dense_of_node(nb.node_id)
                && nd != d
            {
                neighbors.push(nd);
            }
        }
        for nb in proj.in_neighbors(node) {
            if let Some(nd) = idx.dense_of_node(nb.node_id)
                && nd != d
            {
                neighbors.push(nd);
            }
        }
        neighbors.sort_unstable();
        neighbors.dedup();
    }
    Ok(DenseAdjacency { idx, adj })
}

fn count_triangles_at_row(row: usize, adjacency: &DenseAdjacency) -> usize {
    let mut count = 0;
    let neighbors = adjacency.neighbors(row);
    for i in 0..neighbors.len() {
        // Pair walk: pick `v < w` (already sorted), check `adj[v]` for w.
        // The `binary_search` gives O(log d) per pair → O(d² log d) per
        // vertex worst case. Acceptable per §J Q12.
        for j in (i + 1)..neighbors.len() {
            let v = neighbors[i];
            let w = neighbors[j];
            if adjacency.neighbors(v as usize).binary_search(&w).is_ok() {
                count += 1;
            }
        }
    }
    count
}

fn sort_triangle_count_results(mut result: Vec<(NodeId, usize)>) -> Vec<(NodeId, usize)> {
    // §E27: DESC by count with NodeId ASC tie-break. Explicit comparator per
    // `feedback_dijkstra_tie_break_needs_both_rules`.
    result.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.get().cmp(&b.0.get())));
    result
}
