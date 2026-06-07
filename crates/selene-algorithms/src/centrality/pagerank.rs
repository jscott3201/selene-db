//! PageRank via power iteration with damping factor.
//!
//! Standard Brin-Page formulation, with optional personalized restart
//! distribution `p`:
//!   `new[v] = (1 - damping) * p[v] + damping * Σ score[u] / out_degree(u)`
//! for each `u` ∈ in_neighbors(v). Dangling nodes (out_degree = 0)
//! redistribute their score to `p`.
//!
//! State arrays sized by live count via `RowIndex` (§E20). Result sorted
//! DESC by score with NodeId ASC tie-break (§E21).

use rayon::prelude::*;
use selene_core::{CancellationChecker, NodeId};

use crate::error::{AlgorithmAborted, check_algorithm, check_algorithm_stride};
use crate::parallel::{ParallelRunner, Parallelism};
use crate::projection::GraphProjection;

/// Caller-supplied PageRank configuration.
///
/// Literal construction via struct expression is part of the ergonomic
/// contract (matching `ProjectionConfig`).
#[derive(Debug, Clone)]
pub struct PageRankConfig {
    /// Damping factor — probability of following an out-edge versus random
    /// teleport. Typical value 0.85. Must be finite and in `[0.0, 1.0)`;
    /// the exclusive upper bound preserves the teleport floor that gives the
    /// power iteration a convergence guarantee. Callers are responsible for
    /// validating before passing.
    pub damping: f64,
    /// Maximum power-iteration count. Algorithm terminates earlier when
    /// `max |new[v] - score[v]| < tolerance` across all v. `0` returns the
    /// initial restart distribution immediately.
    pub max_iter: usize,
    /// Convergence tolerance. `0.0` runs all `max_iter` iterations
    /// regardless per §O.10.
    pub tolerance: f64,
    /// Requested parallel execution policy.
    pub parallelism: Parallelism,
    /// Optional personalized restart distribution as seed-node weights.
    ///
    /// `None` keeps the uniform PageRank behavior. `Some` weights are normalized
    /// across projection nodes before iteration, duplicate seeds are summed, and
    /// non-seed projection nodes receive `0.0` restart probability. Callers should
    /// supply finite, non-negative weights with at least one positive in-projection
    /// seed.
    pub personalization: Option<Vec<(NodeId, f64)>>,
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
        Parallelism::Sequential | Parallelism::Auto => pagerank_sequential(proj, config, checker),
        Parallelism::Threads(_) => pagerank_parallel(proj, config, checker),
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
    let idx = proj.row_index();
    if idx.is_empty() {
        return Ok(Vec::new());
    }
    let n_usize = idx.len();
    let personalization = personalization_distribution(idx, n_usize, &config);

    let mut scores: Vec<f64> = personalization.clone();
    let mut new_scores: Vec<f64> = vec![0.0; n_usize];

    // Pre-cache out-neighbor dense lists once. PageRank touches every edge
    // every iteration; caching avoids re-walking the projection.
    let mut out_neighbors_dense: Vec<Vec<u32>> = Vec::with_capacity(n_usize);
    let mut rows_since_check = 0usize;
    for d in 0..n_usize as u32 {
        check_algorithm_stride(checker, &mut rows_since_check)?;
        let node = idx.node_id_of(d);
        let neighbors: Vec<u32> = proj.out_neighbors(node).iter().map(|nb| nb.dense).collect();
        out_neighbors_dense.push(neighbors);
    }

    for _ in 0..config.max_iter {
        check_algorithm(checker)?;
        // Seed new_scores with restart probability. With no personalization,
        // `p` is uniform, preserving classic PageRank behavior.
        let restart_mass = 1.0 - config.damping;
        for (slot, restart_probability) in new_scores.iter_mut().zip(personalization.iter()) {
            *slot = restart_mass * restart_probability;
        }

        // Accumulate damping * score[u] / out_degree(u) contributions for
        // nodes with out-edges; collect dangling mass for a single bulk
        // redistribute pass at the end of the iteration.
        //
        // Why bulk-apply dangling mass (PR #60 Codex P1):
        // Naively iterating `for slot in new_scores.iter_mut()` per dangling
        // node yields O(N * D) per iteration — quadratic when D ≈ N on
        // sink-heavy graphs. Accumulating total dangling mass and applying
        // `damping * dangling_mass * p[v]` once to every node is
        // mathematically equivalent (sums commute) and runs in O(N + E)
        // regardless of D.
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
            let redistributed_mass = config.damping * dangling_mass;
            for (slot, restart_probability) in new_scores.iter_mut().zip(personalization.iter()) {
                *slot += redistributed_mass * restart_probability;
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
    let idx = proj.row_index();
    if idx.is_empty() {
        return Ok(Vec::new());
    }
    let n_usize = idx.len();
    let personalization = personalization_distribution(idx, n_usize, &config);

    let mut scores: Vec<f64> = personalization.clone();
    let mut new_scores: Vec<f64> = vec![0.0; n_usize];

    let mut in_neighbors_dense: Vec<Vec<u32>> = Vec::with_capacity(n_usize);
    let mut out_degree_dense: Vec<usize> = vec![0; n_usize];
    let mut dangling_rows: Vec<u32> = Vec::new();

    let mut rows_since_check = 0usize;
    for d in 0..n_usize as u32 {
        check_algorithm_stride(checker, &mut rows_since_check)?;
        let node = idx.node_id_of(d);
        let out_degree = proj.out_degree(node);
        out_degree_dense[d as usize] = out_degree;
        if out_degree == 0 {
            dangling_rows.push(d);
        }

        let in_neighbors: Vec<u32> = proj.in_neighbors(node).iter().map(|nb| nb.dense).collect();
        in_neighbors_dense.push(in_neighbors);
    }

    let runner = ParallelRunner::new(config.parallelism)
        .expect("ParallelRunner builds for valid parallelism");
    runner.install(|| -> Result<(), AlgorithmAborted> {
        for _ in 0..config.max_iter {
            check_algorithm(checker)?;
            let dangling_mass: f64 = dangling_rows.par_iter().map(|&u| scores[u as usize]).sum();
            let restart_mass = (1.0 - config.damping) + (config.damping * dangling_mass);

            new_scores.par_iter_mut().enumerate().for_each(|(v, slot)| {
                let mut inbound = 0.0;
                for &u in &in_neighbors_dense[v] {
                    let out_degree = out_degree_dense[u as usize];
                    debug_assert!(out_degree > 0);
                    inbound += scores[u as usize] / out_degree as f64;
                }
                *slot = (restart_mass * personalization[v]) + (config.damping * inbound);
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

fn personalization_distribution(
    idx: &crate::projection::RowIndex,
    n_usize: usize,
    config: &PageRankConfig,
) -> Vec<f64> {
    let uniform = || vec![1.0 / n_usize as f64; n_usize];
    let Some(seeds) = &config.personalization else {
        return uniform();
    };

    let mut distribution = vec![0.0; n_usize];
    let mut total = 0.0;
    for &(node, weight) in seeds {
        debug_assert!(
            weight.is_finite() && weight >= 0.0,
            "personalization weights must be finite and non-negative"
        );
        if !weight.is_finite() || weight <= 0.0 {
            continue;
        }
        if let Some(dense) = idx.dense_of_node(node) {
            distribution[dense as usize] += weight;
            total += weight;
        }
    }
    debug_assert!(
        total > 0.0,
        "personalization must include at least one positive in-projection seed"
    );
    if total <= 0.0 {
        return uniform();
    }

    for slot in &mut distribution {
        *slot /= total;
    }
    distribution
}
