//! PageRank via power iteration with damping factor.
//!
//! Standard Brin-Page formulation:
//!   `new[v] = (1 - damping) / N + damping * Σ score[u] / out_degree(u)`
//! for each `u` ∈ in_neighbors(v). Dangling nodes (out_degree = 0)
//! redistribute their score evenly across all N nodes per spec 16 §E23.
//!
//! State arrays sized by live count via `RowIndex` (§E20). Result sorted
//! DESC by score with NodeId ASC tie-break (§E21).

use rayon::prelude::*;
use selene_core::{CancellationChecker, NodeId};

use crate::error::{AlgorithmAborted, check_algorithm, check_algorithm_stride};
use crate::parallel::{ParallelRunner, Parallelism};
use crate::projection::GraphProjection;
use crate::structural::RowIndex;

/// Caller-supplied PageRank configuration (spec 16 §E22 — no in-crate
/// defaults).
#[derive(Debug, Clone, Copy)]
pub struct PageRankConfig {
    /// Damping factor — probability of following an out-edge versus random
    /// teleport. Typical value 0.85. Must be finite and in `[0.0, 1.0)`;
    /// the exclusive upper bound preserves the teleport floor that gives the
    /// power iteration a convergence guarantee. Callers are responsible for
    /// validating before passing.
    pub damping: f64,
    /// Maximum power-iteration count. Algorithm terminates earlier when
    /// `max |new[v] - score[v]| < tolerance` across all v. `0` returns the
    /// initial uniform `1/N` scores immediately per §E22 / §O.11.
    pub max_iter: usize,
    /// Convergence tolerance. `0.0` runs all `max_iter` iterations
    /// regardless per §O.10.
    pub tolerance: f64,
    /// Requested parallel execution policy.
    pub parallelism: Parallelism,
}

/// Compute PageRank scores for every node in the projection.
///
/// Returns `(NodeId, score)` pairs sorted **DESC by score** with **NodeId
/// ASC** tie-break on equal scores (spec 16 §E21). Empty projection →
/// `vec![]`.
///
/// The algorithm is **infallible** — no error variant is needed because
/// inputs cannot be malformed (damping/tolerance values are caller-validated;
/// non-finite results would only occur on caller-supplied invalid configs).
#[must_use]
pub fn pagerank(proj: &GraphProjection, config: PageRankConfig) -> Vec<(NodeId, f64)> {
    pagerank_with_checker(proj, config, CancellationChecker::disabled())
        .expect("disabled cancellation checker never aborts")
}

/// Compute PageRank with cooperative cancellation checkpoints.
pub fn pagerank_with_checker(
    proj: &GraphProjection,
    config: PageRankConfig,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, f64)>, AlgorithmAborted> {
    match config.parallelism {
        Parallelism::Sequential => pagerank_sequential(proj, config, checker),
        Parallelism::Auto | Parallelism::Threads(_) => pagerank_parallel(proj, config, checker),
    }
}

fn pagerank_sequential(
    proj: &GraphProjection,
    config: PageRankConfig,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, f64)>, AlgorithmAborted> {
    debug_assert!(
        config.damping.is_finite() && (0.0..1.0).contains(&config.damping),
        "damping must be finite and in [0.0, 1.0); got {}",
        config.damping
    );
    check_algorithm(checker)?;
    let idx = RowIndex::new(proj);
    if idx.is_empty() {
        return Ok(Vec::new());
    }
    let n_usize = idx.len();
    let n_f64 = n_usize as f64;
    let teleport = (1.0 - config.damping) / n_f64;
    let initial = 1.0 / n_f64;

    let mut scores: Vec<f64> = vec![initial; n_usize];
    let mut new_scores: Vec<f64> = vec![0.0; n_usize];

    // Pre-cache out-neighbor dense lists once. PageRank touches every edge
    // every iteration; caching avoids re-walking the projection.
    let mut out_neighbors_dense: Vec<Vec<u32>> = Vec::with_capacity(n_usize);
    let mut rows_since_check = 0usize;
    for d in 0..n_usize as u32 {
        check_algorithm_stride(checker, &mut rows_since_check)?;
        let node = idx.node_id_of(d);
        let neighbors: Vec<u32> = proj
            .out_neighbors(node)
            .iter()
            .filter_map(|nb| idx.dense_of(node_sparse_row(nb.node_id)))
            .collect();
        out_neighbors_dense.push(neighbors);
    }

    for _ in 0..config.max_iter {
        check_algorithm(checker)?;
        // Seed new_scores with the teleport contribution. Why: the
        // (1-damping)/N teleport is applied once per node (no contribution
        // from dangling-redistribution math needed in seed; dangling falls
        // into the per-node loop below).
        for slot in new_scores.iter_mut() {
            *slot = teleport;
        }

        // Accumulate damping * score[u] / out_degree(u) contributions for
        // nodes with out-edges; collect dangling mass for a single bulk
        // redistribute pass at the end of the iteration.
        //
        // Why bulk-apply dangling mass (PR #60 Codex P1):
        // Naively iterating `for slot in new_scores.iter_mut()` per dangling
        // node yields O(N * D) per iteration — quadratic when D ≈ N on
        // sink-heavy graphs. Accumulating total dangling mass and applying
        // `damping * dangling_mass / N` once to every node is mathematically
        // equivalent (sums commute) and runs in O(N + E) regardless of D.
        let mut dangling_mass = 0.0;
        for u in 0..n_usize {
            let neighbors = &out_neighbors_dense[u];
            if neighbors.is_empty() {
                dangling_mass += scores[u];
            } else {
                let contribution = config.damping * scores[u] / neighbors.len() as f64;
                for &v in neighbors {
                    new_scores[v as usize] += contribution;
                }
            }
        }
        if dangling_mass > 0.0 {
            let spread = config.damping * dangling_mass / n_f64;
            for slot in new_scores.iter_mut() {
                *slot += spread;
            }
        }

        // Convergence check: max |new[v] - score[v]| per §O.5 / §E22.
        let max_diff = new_scores
            .iter()
            .zip(scores.iter())
            .map(|(new, old)| (new - old).abs())
            .fold(0.0_f64, f64::max);

        std::mem::swap(&mut scores, &mut new_scores);

        if max_diff < config.tolerance {
            break;
        }
    }

    // Materialize result + sort DESC by score with NodeId ASC tie-break
    // (§E21 / §O.2). Uses total_cmp for NaN-soundness.
    let mut result: Vec<(NodeId, f64)> = (0..n_usize as u32)
        .map(|d| (idx.node_id_of(d), scores[d as usize]))
        .collect();
    result.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.get().cmp(&b.0.get())));
    Ok(result)
}

fn pagerank_parallel(
    proj: &GraphProjection,
    config: PageRankConfig,
    checker: CancellationChecker<'_>,
) -> Result<Vec<(NodeId, f64)>, AlgorithmAborted> {
    debug_assert!(
        config.damping.is_finite() && (0.0..1.0).contains(&config.damping),
        "damping must be finite and in [0.0, 1.0); got {}",
        config.damping
    );
    check_algorithm(checker)?;
    let idx = RowIndex::new(proj);
    if idx.is_empty() {
        return Ok(Vec::new());
    }
    let n_usize = idx.len();
    let n_f64 = n_usize as f64;
    let teleport = (1.0 - config.damping) / n_f64;
    let initial = 1.0 / n_f64;

    let mut scores: Vec<f64> = vec![initial; n_usize];
    let mut new_scores: Vec<f64> = vec![0.0; n_usize];

    let mut in_neighbors_dense: Vec<Vec<u32>> = Vec::with_capacity(n_usize);
    let mut out_degree_dense: Vec<usize> = vec![0; n_usize];
    let mut dangling_rows: Vec<u32> = Vec::new();

    let mut rows_since_check = 0usize;
    for d in 0..n_usize as u32 {
        check_algorithm_stride(checker, &mut rows_since_check)?;
        let node = idx.node_id_of(d);
        let out_degree = proj
            .out_neighbors(node)
            .iter()
            .filter_map(|nb| idx.dense_of(node_sparse_row(nb.node_id)))
            .count();
        out_degree_dense[d as usize] = out_degree;
        if out_degree == 0 {
            dangling_rows.push(d);
        }

        let in_neighbors: Vec<u32> = proj
            .in_neighbors(node)
            .iter()
            .filter_map(|nb| idx.dense_of(node_sparse_row(nb.node_id)))
            .collect();
        in_neighbors_dense.push(in_neighbors);
    }

    let runner = ParallelRunner::new(config.parallelism)
        .expect("ParallelRunner builds for valid parallelism");
    runner.install(|| -> Result<(), AlgorithmAborted> {
        for _ in 0..config.max_iter {
            check_algorithm(checker)?;
            let dangling_mass: f64 = dangling_rows.par_iter().map(|&u| scores[u as usize]).sum();
            let spread = if dangling_mass > 0.0 {
                config.damping * dangling_mass / n_f64
            } else {
                0.0
            };

            new_scores.par_iter_mut().enumerate().for_each(|(v, slot)| {
                let mut inbound = 0.0;
                for &u in &in_neighbors_dense[v] {
                    let out_degree = out_degree_dense[u as usize];
                    debug_assert!(out_degree > 0);
                    inbound += scores[u as usize] / out_degree as f64;
                }
                *slot = teleport + (config.damping * inbound) + spread;
            });

            let max_diff = new_scores
                .par_iter()
                .zip(scores.par_iter())
                .map(|(new, old)| (new - old).abs())
                .reduce(|| 0.0_f64, f64::max);

            std::mem::swap(&mut scores, &mut new_scores);

            if max_diff < config.tolerance {
                break;
            }
        }
        Ok(())
    })?;

    // Materialize result + sort DESC by score with NodeId ASC tie-break
    // (§E21 / §O.2). Uses total_cmp for NaN-soundness.
    let mut result: Vec<(NodeId, f64)> = (0..n_usize as u32)
        .map(|d| (idx.node_id_of(d), scores[d as usize]))
        .collect();
    result.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.get().cmp(&b.0.get())));
    Ok(result)
}

/// Map a `NodeId` (1-based per selene-graph) to its sparse row index
/// (0-based).
#[inline]
fn node_sparse_row(nid: NodeId) -> u32 {
    (nid.get() - 1) as u32
}
