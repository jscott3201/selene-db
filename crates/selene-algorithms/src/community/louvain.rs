//! Louvain modularity optimization for community detection (single-pass).
//!
//! Each node initially in its own community. Each iteration: for each node u,
//! evaluate modularity gain `Δ` from joining each neighbor community; move u
//! to the candidate with **strictly-largest** gain. Repeat until no node
//! moves, or caller-supplied `max_iter` reached.
//!
//! ## v1.0 scope
//!
//! - Single-pass at `level = 0` (no hierarchical contraction; that ships in
//!   v1.x as multi-level Louvain — the `u32 level` is reserved for forward
//!   compatibility per spec 16 §E27).
//! - Uses projection edge weights (§E04). Defaults to unit weights when the
//!   projection is unweighted.
//! - State arrays sized by live count via `RowIndex` (§E26).
//!
//! ## Determinism (spec 16 §E30)
//!
//! Candidate communities are iterated in **sorted order by community ID** to
//! defuse `HashMap` iteration-order non-determinism. The movement rule is
//! strict `delta > best_delta` (initial `best_delta = 0.0`), so equal-delta
//! candidates do NOT replace — but the sorted iteration makes the algorithm
//! robust to a future `>=` change.

use std::collections::HashMap;

use selene_core::NodeId;

use crate::projection::GraphProjection;
use crate::structural::RowIndex;

/// Compute community assignments via single-pass Louvain modularity.
///
/// Returns `(NodeId, community_id, level)` triples sorted **ASC by NodeId**
/// per spec 16 §E27. `level` is always `0` in v1.0 (reserved for hierarchical
/// Louvain in v1.x). Empty projection → `vec![]`. `max_iter == 0` returns the
/// initial state (each node in its own community).
///
/// Edge weights are projected via `ProjNeighbor::weight` per spec 16 §E04
/// (unit weights when the projection is unweighted).
#[must_use]
pub fn louvain(proj: &GraphProjection, max_iter: usize) -> Vec<(NodeId, u64, u32)> {
    let idx = RowIndex::new(proj);
    if idx.is_empty() {
        return Vec::new();
    }
    let n = idx.len();

    // Total directed weight: Σ over u of Σ over outgoing edges of u of weight.
    // Under selene-db's directed-storage convention this is the absolute
    // weight scale; `weighted_degree[v]` is computed as the sum of out and in
    // weights, so the modularity invariant `Σ d_i = 2 · total_weight` holds.
    let mut total_weight: f64 = 0.0;
    for d in 0..n as u32 {
        let node = idx.node_id_of(d);
        for nb in proj.out_neighbors(node) {
            if idx.dense_of(node_sparse_row(nb.node_id)).is_some() {
                total_weight += nb.weight;
            }
        }
    }
    if total_weight == 0.0 {
        // Avoid div-by-zero in the gain formula. No edges → no movement
        // possible anyway; the value is observationally irrelevant.
        total_weight = 1.0;
    }
    // community[d] = current community ID (encoded as a dense index initially).
    let mut community: Vec<u32> = (0..n as u32).collect();

    // weighted_degree[d] = undirected degree under selene-db's directed-
    // storage convention: sum of outgoing + incoming neighbor weights. A
    // bidirectional undirected edge stored as two directed edges contributes
    // to both sums (correct: undirected degree counts each endpoint once,
    // matching the modularity invariant `Σ d_i = 2 · total_weight` where
    // total_weight is the outgoing-only sum). Edges referencing rows outside
    // the projection scope are skipped (matches §E26 / dense-remap contract).
    // (Brief-back-reviewer-55 P1-rec: clarify undirected interpretation.)
    let mut weighted_degree: Vec<f64> = vec![0.0; n];
    for d in 0..n as u32 {
        let node = idx.node_id_of(d);
        let mut deg = 0.0;
        for nb in proj.out_neighbors(node) {
            if idx.dense_of(node_sparse_row(nb.node_id)).is_some() {
                deg += nb.weight;
            }
        }
        for nb in proj.in_neighbors(node) {
            if idx.dense_of(node_sparse_row(nb.node_id)).is_some() {
                deg += nb.weight;
            }
        }
        weighted_degree[d as usize] = deg;
    }

    // comm_degree_sum[community_id] = Σ weighted_degree[v] for v ∈ community.
    // Kept as a `HashMap` because community IDs are sparse (start = N distinct
    // singletons, may merge down to ≪ N) and the key set changes per move.
    let mut comm_degree_sum: HashMap<u32, f64> = HashMap::with_capacity(n);
    for d in 0..n as u32 {
        let c = community[d as usize];
        *comm_degree_sum.entry(c).or_insert(0.0) += weighted_degree[d as usize];
    }

    // Reused per-node scratch.
    let mut comm_weights: HashMap<u32, f64> = HashMap::new();
    // Sorted-candidate buffer for §E30 determinism. Built fresh per node.
    let mut sorted_candidates: Vec<(u32, f64)> = Vec::new();

    let mut improved = true;
    let mut iter = 0usize;
    while improved && iter < max_iter {
        improved = false;
        iter += 1;

        for d in 0..n as u32 {
            let node = idx.node_id_of(d);
            let idx_d = d as usize;
            let current_comm = community[idx_d];
            let ki = weighted_degree[idx_d];

            comm_weights.clear();
            for nb in proj.out_neighbors(node) {
                if let Some(nd) = idx.dense_of(node_sparse_row(nb.node_id)) {
                    let nb_comm = community[nd as usize];
                    *comm_weights.entry(nb_comm).or_insert(0.0) += nb.weight;
                }
            }
            for nb in proj.in_neighbors(node) {
                if let Some(nd) = idx.dense_of(node_sparse_row(nb.node_id)) {
                    let nb_comm = community[nd as usize];
                    *comm_weights.entry(nb_comm).or_insert(0.0) += nb.weight;
                }
            }

            let ki_in_current = comm_weights.get(&current_comm).copied().unwrap_or(0.0);
            let sigma_current = comm_degree_sum.get(&current_comm).copied().unwrap_or(0.0) - ki;

            // §E30 determinism: iterate candidate communities in sorted order
            // by community ID, not raw HashMap order.
            sorted_candidates.clear();
            sorted_candidates.extend(comm_weights.iter().map(|(&c, &w)| (c, w)));
            sorted_candidates.sort_by_key(|&(c, _)| c);

            let mut best_comm = current_comm;
            let mut best_delta = 0.0f64;

            for &(candidate_comm, ki_in_candidate) in &sorted_candidates {
                if candidate_comm == current_comm {
                    continue;
                }
                let sigma_candidate = comm_degree_sum.get(&candidate_comm).copied().unwrap_or(0.0);

                let delta =
                    compute_modularity_delta(total_weight, ki_in_candidate, ki, sigma_candidate)
                        - compute_modularity_delta(total_weight, ki_in_current, ki, sigma_current);

                if delta > best_delta {
                    best_delta = delta;
                    best_comm = candidate_comm;
                }
            }

            if best_comm != current_comm {
                *comm_degree_sum
                    .get_mut(&current_comm)
                    .expect("current community present") -= ki;
                *comm_degree_sum.entry(best_comm).or_insert(0.0) += ki;
                community[idx_d] = best_comm;
                improved = true;
            }
        }
    }

    // Final result: community IDs are dense indices. Remap to NodeId values
    // (smallest NodeId in each community would be the WCC/SCC convention, but
    // Louvain communities are not "components" in the connectivity sense —
    // donor leaves the dense-index labels as-is, which is fine because they
    // are stable identifiers within a single run). We emit the dense-index
    // form converted to `u64` to match the type signature; if a future brief
    // wants smallest-NodeId-in-community canonicalization, it lives behind a
    // separate canonicalization step (no information loss).
    let mut result: Vec<(NodeId, u64, u32)> = (0..n as u32)
        .map(|d| {
            (
                idx.node_id_of(d),
                idx.node_id_of(community[d as usize]).get(),
                0u32,
            )
        })
        .collect();
    result.sort_by_key(|&(nid, _, _)| nid.get());
    result
}

#[inline]
fn node_sparse_row(nid: NodeId) -> u32 {
    (nid.get() - 1) as u32
}

/// Compute the scalar Louvain gain term for moving a node into one community.
///
/// This pins the formula used by the movement loop:
/// `k_i,in / m - k_i * sigma_tot / (2m^2)`, where `m` is the projection's
/// total directed edge weight, `k_i,in` is the node's incident weight into the
/// candidate community, `k_i` is the node's weighted degree, and `sigma_tot`
/// is the target community's weighted-degree sum. The movement delta is the
/// candidate term minus the current-community term.
pub(crate) fn compute_modularity_delta(
    total_weight: f64,
    ki_in: f64,
    ki: f64,
    sigma_tot: f64,
) -> f64 {
    ki_in / total_weight - (ki * sigma_tot) / (2.0 * total_weight * total_weight)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_modularity_delta_known_answer() {
        let delta = compute_modularity_delta(5.0, 1.0, 2.0, 4.0);

        assert!(
            (delta - 0.04).abs() <= 1.0e-12,
            "expected 0.04, observed {delta}"
        );
    }
}
