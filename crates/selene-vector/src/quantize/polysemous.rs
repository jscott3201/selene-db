//! Polysemous-codes simulated-annealing permutation solver.
//!
//! Polysemous codes are PQ codes re-indexed so that `popcount(σ(i) XOR σ(j))`
//! correlates with `||c_i − c_j||`. After training, the same PQ encoding path
//! produces codes whose Hamming distances rank-correlate with L2 distances,
//! enabling a cheap binary pre-filter in front of ADC scoring.
//!
//! The training routine is per-subspace, deterministic given seed, zero-dep
//! (uses `fastrand` only), and applied to the codebook centroids in place.
//! The wire format is unchanged from a non-polysemous codebook of the same
//! parameters; an archive flag declares whether σ was applied so recovery can
//! reject config drift.
//!
//! See Spec 17 V105–V117 for persisted invariants.

use crate::VectorError;

/// Per-subspace deterministic seed. XORed with the subspace index so each
/// subspace's simulated-annealing PRNG is independent and reproducible.
pub(crate) const POLYSEMOUS_TRAIN_SEED: u64 = 0xB69E_0001_u64;

/// Cap on simulated-annealing swap attempts per subspace.
const SA_ITERATION_CAP: usize = 100_000;

/// Terminate the per-subspace SA loop once this many consecutive swap
/// attempts are rejected with no acceptance.
const SA_NO_PROGRESS_CAP: usize = 10_000;

/// Multiplicative cooling rate applied every `SA_COOLING_INTERVAL` swaps.
const SA_COOLING_RATE: f32 = 0.95;

/// Number of swap attempts between cooling steps.
const SA_COOLING_INTERVAL: usize = 1_000;

/// Train per-subspace polysemous permutations against the post-OPQ codebook.
///
/// Each output `σ_s` is a permutation of `0..k`. Per-subspace SA uses
/// `seed XOR subspace_index` so subspaces train independently and the global
/// `seed` argument plus the codebook bytes alone determine the result
/// (V105).
///
/// `k` must be in `1..=256` (one byte per code). The internal solver supports
/// any `k <= 256` so unit tests can exercise small cases (V107).
///
/// # Errors
///
/// Returns [`VectorError::PolysemousTrainingFailed`] when the centroid buffer
/// length disagrees with `m * k * subspace_dim`, when `k` exceeds 256, when
/// `m == 0`, or when SA produces a non-permutation (defensive — would only
/// trigger on a programming error).
pub(crate) fn train(
    centroids: &[f32],
    m: usize,
    k: usize,
    subspace_dim: usize,
    seed: u64,
    context: &'static str,
) -> Result<Vec<Vec<u8>>, VectorError> {
    if m == 0 {
        return Err(VectorError::PolysemousTrainingFailed {
            context,
            reason: "polysemous m_subspaces must be greater than zero".into(),
        });
    }
    if k == 0 || k > 256 {
        return Err(VectorError::PolysemousTrainingFailed {
            context,
            reason: format!("polysemous k must be in 1..=256, observed {k}"),
        });
    }
    let expected = m
        .checked_mul(k)
        .and_then(|value| value.checked_mul(subspace_dim))
        .ok_or_else(|| VectorError::PolysemousTrainingFailed {
            context,
            reason: "polysemous centroid length overflow".into(),
        })?;
    if centroids.len() != expected {
        return Err(VectorError::PolysemousTrainingFailed {
            context,
            reason: format!(
                "polysemous centroid length {} disagrees with m*k*subspace_dim {expected}",
                centroids.len()
            ),
        });
    }

    let mut sigmas = Vec::with_capacity(m);
    for subspace_index in 0..m {
        let start = subspace_index * k * subspace_dim;
        let subspace = &centroids[start..start + k * subspace_dim];
        let subspace_seed = seed ^ (subspace_index as u64);
        let sigma = solve_subspace(subspace, k, subspace_dim, subspace_seed);
        if !is_valid_permutation(&sigma, k) {
            return Err(VectorError::PolysemousTrainingFailed {
                context,
                reason: format!(
                    "polysemous subspace {subspace_index} solver produced a non-permutation"
                ),
            });
        }
        sigmas.push(sigma);
    }
    Ok(sigmas)
}

/// Apply per-subspace permutations to a codebook in place.
///
/// `centroids[start + σ_s(i) * subspace_dim .. + subspace_dim]` receives the
/// pre-swap centroid at index `i`. Encoding against the permuted codebook
/// produces polysemous code bytes.
///
/// # Errors
///
/// Returns [`VectorError::PolysemousTrainingFailed`] when shapes disagree or
/// any σ is not a valid permutation.
pub(crate) fn apply_permutation(
    centroids: &mut [f32],
    sigmas: &[Vec<u8>],
    m: usize,
    k: usize,
    subspace_dim: usize,
    context: &'static str,
) -> Result<(), VectorError> {
    let expected = m
        .checked_mul(k)
        .and_then(|value| value.checked_mul(subspace_dim))
        .ok_or_else(|| VectorError::PolysemousTrainingFailed {
            context,
            reason: "polysemous centroid length overflow".into(),
        })?;
    if centroids.len() != expected {
        return Err(VectorError::PolysemousTrainingFailed {
            context,
            reason: format!(
                "polysemous apply: centroid length {} disagrees with m*k*subspace_dim {expected}",
                centroids.len()
            ),
        });
    }
    if sigmas.len() != m {
        return Err(VectorError::PolysemousTrainingFailed {
            context,
            reason: format!(
                "polysemous apply: sigma count {} disagrees with m {m}",
                sigmas.len()
            ),
        });
    }
    let mut tmp = vec![0.0f32; k * subspace_dim];
    for (subspace_index, sigma) in sigmas.iter().enumerate() {
        if sigma.len() != k {
            return Err(VectorError::PolysemousTrainingFailed {
                context,
                reason: format!(
                    "polysemous apply: subspace {subspace_index} sigma length {} disagrees with k {k}",
                    sigma.len()
                ),
            });
        }
        if !is_valid_permutation(sigma, k) {
            return Err(VectorError::PolysemousTrainingFailed {
                context,
                reason: format!(
                    "polysemous apply: subspace {subspace_index} sigma is not a permutation"
                ),
            });
        }
        let start = subspace_index * k * subspace_dim;
        // tmp[σ(i)] = centroids[start + i] — the centroid at codebook slot
        // `i` is reassigned to byte index `σ(i)` so encoding now produces
        // polysemous codes against the same encoding path.
        for (i, &sigma_i) in sigma.iter().enumerate() {
            let dst = sigma_i as usize;
            let src_offset = start + i * subspace_dim;
            let dst_offset = dst * subspace_dim;
            tmp[dst_offset..dst_offset + subspace_dim]
                .copy_from_slice(&centroids[src_offset..src_offset + subspace_dim]);
        }
        centroids[start..start + k * subspace_dim].copy_from_slice(&tmp);
    }
    Ok(())
}

/// Per-subspace simulated annealing on a permutation of `0..k`.
fn solve_subspace(centroids: &[f32], k: usize, subspace_dim: usize, seed: u64) -> Vec<u8> {
    let dists = pairwise_l2(centroids, k, subspace_dim);
    let weights = pairwise_weights(&dists, k);
    let alpha = compute_alpha(&dists, k);
    let initial_temperature = mean_pairwise_distance(&dists, k).max(f32::EPSILON);

    let mut rng = fastrand::Rng::with_seed(seed);
    let mut sigma: Vec<u8> = (0..k as u32).map(|value| value as u8).collect();
    let mut loss = total_loss(&dists, &weights, &sigma, alpha, k);
    let mut best_sigma = sigma.clone();
    let mut best_loss = loss;
    let mut no_accept = 0usize;
    let mut temperature = initial_temperature;

    for iter in 0..SA_ITERATION_CAP {
        // The candidate move is a transposition. Same-index swaps are no-ops
        // and intentionally skipped without consuming a "no accept" slot —
        // they are just RNG noise, not rejections.
        let a = rng.usize(0..k);
        let b = rng.usize(0..k);
        if a == b {
            if (iter + 1) % SA_COOLING_INTERVAL == 0 {
                temperature *= SA_COOLING_RATE;
            }
            continue;
        }
        let delta = swap_delta(&dists, &weights, &sigma, a, b, alpha, k);
        let accept = if delta <= 0.0 {
            true
        } else if temperature <= 0.0 {
            false
        } else {
            rng.f32() < (-delta / temperature).exp()
        };
        if accept {
            sigma.swap(a, b);
            loss += delta;
            if loss < best_loss {
                best_loss = loss;
                best_sigma.copy_from_slice(&sigma);
            }
            no_accept = 0;
        } else {
            no_accept += 1;
            if no_accept >= SA_NO_PROGRESS_CAP {
                break;
            }
        }
        if (iter + 1) % SA_COOLING_INTERVAL == 0 {
            temperature *= SA_COOLING_RATE;
        }
    }
    best_sigma
}

fn pairwise_l2(centroids: &[f32], k: usize, subspace_dim: usize) -> Vec<f32> {
    let mut dists = vec![0.0f32; k * k];
    for i in 0..k {
        for j in i + 1..k {
            let mut sum_sq = 0.0f32;
            for d in 0..subspace_dim {
                let diff = centroids[i * subspace_dim + d] - centroids[j * subspace_dim + d];
                sum_sq += diff * diff;
            }
            let dist = sum_sq.sqrt();
            dists[i * k + j] = dist;
            dists[j * k + i] = dist;
        }
    }
    dists
}

fn pairwise_weights(dists: &[f32], k: usize) -> Vec<f32> {
    let mut weights = vec![0.0f32; k * k];
    for i in 0..k {
        for j in i + 1..k {
            let weight = 1.0 / (1.0 + dists[i * k + j]);
            weights[i * k + j] = weight;
            weights[j * k + i] = weight;
        }
    }
    weights
}

fn compute_alpha(dists: &[f32], k: usize) -> f32 {
    let mut sum_l2 = 0.0f32;
    let mut sum_hamming = 0.0f32;
    for i in 0..k {
        for j in i + 1..k {
            sum_l2 += dists[i * k + j];
            sum_hamming += ((i ^ j) as u32).count_ones() as f32;
        }
    }
    if sum_hamming == 0.0 {
        1.0
    } else {
        sum_l2 / sum_hamming
    }
}

fn mean_pairwise_distance(dists: &[f32], k: usize) -> f32 {
    let mut sum = 0.0f32;
    let mut count = 0u32;
    for i in 0..k {
        for j in i + 1..k {
            sum += dists[i * k + j];
            count += 1;
        }
    }
    if count == 0 { 0.0 } else { sum / count as f32 }
}

fn total_loss(dists: &[f32], weights: &[f32], sigma: &[u8], alpha: f32, k: usize) -> f32 {
    let mut loss = 0.0f32;
    for i in 0..k {
        for j in i + 1..k {
            let hamming = (sigma[i] ^ sigma[j]).count_ones() as f32;
            let diff = dists[i * k + j] - alpha * hamming;
            loss += weights[i * k + j] * diff * diff;
        }
    }
    loss
}

/// Incremental loss change for swapping σ entries at positions `a` and `b`.
/// Only edges incident to `a` or `b` change; the (a, b) edge itself is
/// unchanged because `σ(a) XOR σ(b)` is symmetric. O(k) per swap (vs. O(k²)
/// for full recomputation).
fn swap_delta(
    dists: &[f32],
    weights: &[f32],
    sigma: &[u8],
    a: usize,
    b: usize,
    alpha: f32,
    k: usize,
) -> f32 {
    let sigma_a_old = sigma[a];
    let sigma_b_old = sigma[b];
    let sigma_a_new = sigma_b_old;
    let sigma_b_new = sigma_a_old;
    let mut delta = 0.0f32;
    for c in 0..k {
        if c == a || c == b {
            continue;
        }
        let sigma_c = sigma[c];
        let dist_ac = dists[a * k + c];
        let dist_bc = dists[b * k + c];
        let weight_ac = weights[a * k + c];
        let weight_bc = weights[b * k + c];
        let h_ac_old = (sigma_a_old ^ sigma_c).count_ones() as f32;
        let h_bc_old = (sigma_b_old ^ sigma_c).count_ones() as f32;
        let h_ac_new = (sigma_a_new ^ sigma_c).count_ones() as f32;
        let h_bc_new = (sigma_b_new ^ sigma_c).count_ones() as f32;
        let before_ac = dist_ac - alpha * h_ac_old;
        let before_bc = dist_bc - alpha * h_bc_old;
        let after_ac = dist_ac - alpha * h_ac_new;
        let after_bc = dist_bc - alpha * h_bc_new;
        delta += weight_ac * (after_ac * after_ac - before_ac * before_ac);
        delta += weight_bc * (after_bc * after_bc - before_bc * before_bc);
    }
    delta
}

fn is_valid_permutation(sigma: &[u8], k: usize) -> bool {
    if sigma.len() != k {
        return false;
    }
    let mut seen = vec![false; k];
    for &value in sigma {
        let index = value as usize;
        if index >= k || seen[index] {
            return false;
        }
        seen[index] = true;
    }
    true
}

#[cfg(test)]
pub(crate) fn total_loss_for_tests(
    centroids: &[f32],
    sigma: &[u8],
    k: usize,
    subspace_dim: usize,
) -> f32 {
    let dists = pairwise_l2(centroids, k, subspace_dim);
    let weights = pairwise_weights(&dists, k);
    let alpha = compute_alpha(&dists, k);
    total_loss(&dists, &weights, sigma, alpha, k)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear_centroids_1d(values: &[f32]) -> Vec<f32> {
        values.to_vec()
    }

    /// k=4, m=1, subspace_dim=1, centroids at exponentially-spaced positions.
    /// A Gray-coded σ (mapping linear order to a sequence whose byte XORs
    /// have low popcount for adjacent elements) MUST strictly beat the
    /// identity σ on the polysemous loss.
    ///
    /// Codex F4 hardening: a no-op SA solver that returns `[0,1,2,3]` would
    /// pass `solver_loss <= identity_loss` trivially. This test pins:
    ///   (a) sigma is a valid permutation,
    ///   (b) sigma is NOT the identity (solver actually moved),
    ///   (c) solver_loss is STRICTLY less than identity_loss, and
    ///   (d) solver_loss matches the global optimum over all 4! = 24
    ///       permutations (a 4-element brute force is tractable).
    #[test]
    fn polysemous_known_answer_k4_beats_identity() {
        let centroids = linear_centroids_1d(&[0.0, 1.0, 3.0, 7.0]);
        let identity: Vec<u8> = vec![0, 1, 2, 3];
        let identity_loss = total_loss_for_tests(&centroids, &identity, 4, 1);

        let sigmas = train(
            &centroids,
            1,
            4,
            1,
            POLYSEMOUS_TRAIN_SEED,
            "test_known_answer",
        )
        .unwrap();
        let solver_loss = total_loss_for_tests(&centroids, &sigmas[0], 4, 1);

        assert!(is_valid_permutation(&sigmas[0], 4));
        assert_ne!(
            sigmas[0], identity,
            "SA must produce a non-identity permutation on this exponentially-spaced corpus"
        );
        assert!(
            solver_loss < identity_loss,
            "solver loss {solver_loss} must be STRICTLY less than identity loss {identity_loss}"
        );

        // Brute-force the global optimum and confirm SA matches it.
        let mut best_loss = f32::INFINITY;
        let perms = enumerate_permutations(4);
        for perm in &perms {
            let loss = total_loss_for_tests(&centroids, perm, 4, 1);
            if loss < best_loss {
                best_loss = loss;
            }
        }
        let tolerance = (best_loss.abs() * 1.0e-5).max(1.0e-6);
        assert!(
            (solver_loss - best_loss).abs() < tolerance,
            "solver loss {solver_loss} should match global optimum {best_loss} within {tolerance}"
        );
    }

    /// Codex F4 second half: §G correlation/improvement floor. The §G
    /// contract is "polysemous σ produces measurable rank improvement over
    /// identity for L2-vs-Hamming on a realistic centroid corpus." Use
    /// 8 centroids on a 1-D line to keep enumeration tractable while still
    /// having an interesting permutation space (40320 perms).
    #[test]
    fn polysemous_improvement_over_identity_meets_floor() {
        let centroids = linear_centroids_1d(&[0.0, 1.0, 3.0, 7.0, 8.5, 11.0, 12.7, 16.2]);
        let identity: Vec<u8> = (0u8..8).collect();
        let identity_loss = total_loss_for_tests(&centroids, &identity, 8, 1);
        let sigmas = train(
            &centroids,
            1,
            8,
            1,
            POLYSEMOUS_TRAIN_SEED,
            "test_improvement_floor",
        )
        .unwrap();
        let solver_loss = total_loss_for_tests(&centroids, &sigmas[0], 8, 1);
        assert_ne!(sigmas[0], identity);
        // Improvement >= 10% — well below FAISS's typical 30–50% for
        // structured data but tight enough that a degenerate solver fails.
        let improvement = (identity_loss - solver_loss) / identity_loss.abs().max(1.0e-9);
        assert!(
            improvement >= 0.10,
            "polysemous improvement {improvement:.4} must be at least 10% (identity_loss={identity_loss}, solver_loss={solver_loss})"
        );
    }

    fn enumerate_permutations(k: usize) -> Vec<Vec<u8>> {
        fn recurse(prefix: &mut Vec<u8>, remaining: &mut Vec<u8>, out: &mut Vec<Vec<u8>>) {
            if remaining.is_empty() {
                out.push(prefix.clone());
                return;
            }
            for i in 0..remaining.len() {
                let v = remaining.remove(i);
                prefix.push(v);
                recurse(prefix, remaining, out);
                prefix.pop();
                remaining.insert(i, v);
            }
        }
        let mut prefix = Vec::with_capacity(k);
        let mut remaining: Vec<u8> = (0..k as u8).collect();
        let mut out = Vec::new();
        recurse(&mut prefix, &mut remaining, &mut out);
        out
    }

    #[test]
    fn polysemous_training_is_deterministic_given_seed() {
        let centroids = linear_centroids_1d(&[0.0, 1.0, 3.0, 7.0, 8.5, 11.0, 12.7, 16.2]);
        let left = train(&centroids, 1, 8, 1, POLYSEMOUS_TRAIN_SEED, "test_left").unwrap();
        let right = train(&centroids, 1, 8, 1, POLYSEMOUS_TRAIN_SEED, "test_right").unwrap();
        assert_eq!(left, right);
    }

    #[test]
    fn polysemous_seed_pinned_at_b69e_0001() {
        assert_eq!(POLYSEMOUS_TRAIN_SEED, 0xB69E_0001_u64);
    }

    #[test]
    fn polysemous_per_subspace_rngs_are_independent() {
        let mut centroids = linear_centroids_1d(&[0.0, 1.0, 3.0, 7.0]);
        centroids.extend_from_slice(&[0.0, 1.0, 3.0, 7.0]);
        let sigmas = train(
            &centroids,
            2,
            4,
            1,
            POLYSEMOUS_TRAIN_SEED,
            "test_independence",
        )
        .unwrap();
        assert!(
            sigmas[0] != vec![0, 1, 2, 3] || sigmas[1] != vec![0, 1, 2, 3],
            "at least one subspace must move off identity for this corpus"
        );
    }

    #[test]
    fn polysemous_returns_valid_permutation() {
        let centroids = linear_centroids_1d(&[0.0, 1.0, 3.0, 7.0]);
        let sigmas = train(&centroids, 1, 4, 1, POLYSEMOUS_TRAIN_SEED, "test_valid").unwrap();
        assert!(is_valid_permutation(&sigmas[0], 4));
        let mut sorted = sigmas[0].clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1, 2, 3]);
    }

    #[test]
    fn polysemous_apply_permutation_round_trips_to_identity() {
        let mut centroids = vec![0.0_f32, 1.0, 3.0, 7.0];
        let original = centroids.clone();
        let identity: Vec<u8> = vec![0, 1, 2, 3];
        apply_permutation(&mut centroids, &[identity], 1, 4, 1, "test_apply_identity").unwrap();
        assert_eq!(centroids, original);
    }

    #[test]
    fn polysemous_apply_permutation_reorders_centroids() {
        let mut centroids = vec![10.0_f32, 20.0, 30.0, 40.0];
        let sigma = vec![3, 0, 1, 2];
        apply_permutation(&mut centroids, &[sigma], 1, 4, 1, "test_apply").unwrap();
        assert_eq!(centroids, vec![20.0, 30.0, 40.0, 10.0]);
    }

    #[test]
    fn apply_permutation_with_2d_subspace() {
        let mut centroids = vec![10.0_f32, 11.0, 20.0, 21.0, 30.0, 31.0];
        let sigma = vec![2, 0, 1];

        apply_permutation(&mut centroids, &[sigma], 1, 3, 2, "test_apply_2d").unwrap();

        assert_eq!(centroids, vec![20.0, 21.0, 30.0, 31.0, 10.0, 11.0]);
    }

    #[test]
    fn polysemous_apply_permutation_rejects_non_permutation() {
        let mut centroids = vec![1.0_f32, 2.0, 3.0, 4.0];
        let invalid = vec![0u8, 0, 1, 2];
        let err = apply_permutation(&mut centroids, &[invalid], 1, 4, 1, "test_invalid")
            .expect_err("non-permutation rejected");
        assert!(matches!(err, VectorError::PolysemousTrainingFailed { .. }));
    }

    #[test]
    fn polysemous_apply_permutation_rejects_size_mismatch() {
        let mut centroids = vec![1.0_f32, 2.0, 3.0];
        let sigma = vec![0u8, 1, 2, 3];
        let err = apply_permutation(&mut centroids, &[sigma], 1, 4, 1, "test_size")
            .expect_err("size mismatch rejected");
        assert!(matches!(err, VectorError::PolysemousTrainingFailed { .. }));
    }

    #[test]
    fn polysemous_train_rejects_centroid_length_mismatch() {
        let centroids = vec![0.0_f32, 1.0, 2.0];
        let err = train(
            &centroids,
            1,
            4,
            1,
            POLYSEMOUS_TRAIN_SEED,
            "test_centroid_mismatch",
        )
        .expect_err("length mismatch rejected");
        assert!(matches!(err, VectorError::PolysemousTrainingFailed { .. }));
    }

    #[test]
    fn polysemous_train_rejects_k_out_of_range() {
        let centroids = vec![0.0_f32; 257];
        let err = train(
            &centroids,
            1,
            257,
            1,
            POLYSEMOUS_TRAIN_SEED,
            "test_k_overflow",
        )
        .expect_err("k > 256 rejected");
        assert!(matches!(err, VectorError::PolysemousTrainingFailed { .. }));
    }

    #[test]
    fn polysemous_train_rejects_zero_subspaces() {
        let err =
            train(&[], 0, 4, 1, POLYSEMOUS_TRAIN_SEED, "test_zero_m").expect_err("m == 0 rejected");
        assert!(matches!(err, VectorError::PolysemousTrainingFailed { .. }));
    }

    #[test]
    fn polysemous_constant_centroids_returns_valid_permutation() {
        let centroids = vec![5.0_f32; 4];
        let sigmas = train(&centroids, 1, 4, 1, POLYSEMOUS_TRAIN_SEED, "test_constant").unwrap();
        assert!(is_valid_permutation(&sigmas[0], 4));
    }

    #[test]
    fn polysemous_correlation_improves_after_training() {
        let mut centroids = Vec::with_capacity(8 * 2);
        for i in 0..8 {
            let angle = (i as f32) * std::f32::consts::TAU / 8.0;
            centroids.push(angle.cos());
            centroids.push(angle.sin());
        }
        let identity: Vec<u8> = (0..8).collect();
        let identity_loss = total_loss_for_tests(&centroids, &identity, 8, 2);

        let sigmas = train(
            &centroids,
            1,
            8,
            2,
            POLYSEMOUS_TRAIN_SEED,
            "test_correlation",
        )
        .unwrap();
        let solver_loss = total_loss_for_tests(&centroids, &sigmas[0], 8, 2);

        assert!(
            solver_loss <= identity_loss,
            "polysemous training should not regress vs identity: solver {solver_loss}, identity {identity_loss}"
        );
    }
}
