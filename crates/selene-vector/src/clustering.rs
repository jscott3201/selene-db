//! Deterministic scalar k-means helpers shared by PQ and IVF.

use crate::VectorError;

const MAX_LLOYD_ITERS: usize = 25;
const LLOYD_DRIFT_TOL: f32 = 1.0e-4;

/// Train a full-dimensional k-means codebook.
pub(crate) fn kmeans_train(
    rows: &[&[f32]],
    dim: usize,
    k: usize,
    seed: u64,
    context: &'static str,
) -> Result<Vec<f32>, VectorError> {
    if rows.is_empty() {
        return Err(VectorError::IvfTrainingFailed {
            context,
            reason: "cannot train k-means with zero rows".into(),
        });
    }
    if k == 0 || k > rows.len() {
        return Err(VectorError::IvfTrainingFailed {
            context,
            reason: format!("k must be in 1..={}, observed {k}", rows.len()),
        });
    }
    for (index, row) in rows.iter().enumerate() {
        if row.len() != dim {
            return Err(VectorError::IvfTrainingFailed {
                context,
                reason: format!("row {index} dimension mismatch: {} != {dim}", row.len()),
            });
        }
    }
    let mut rng = fastrand::Rng::with_seed(seed);
    Ok(kmeans_train_subspace(rows, 0, dim, k, &mut rng))
}

/// Train one fixed-width subspace using the shared k-means implementation.
pub(crate) fn kmeans_train_subspace(
    rows: &[&[f32]],
    start: usize,
    subdim: usize,
    k: usize,
    rng: &mut fastrand::Rng,
) -> Vec<f32> {
    let mut centroids = kmeans_pp_init(rows, start, subdim, k, rng);
    let mut assignments = vec![0_usize; rows.len()];
    for _ in 0..MAX_LLOYD_ITERS {
        let mut sums = vec![0.0; k * subdim];
        let mut counts = vec![0_usize; k];
        for (row_idx, row) in rows.iter().enumerate() {
            let slice = &row[start..start + subdim];
            let assigned = nearest_centroid(slice, &centroids, k, subdim);
            assignments[row_idx] = assigned;
            counts[assigned] += 1;
            let sum = &mut sums[assigned * subdim..(assigned + 1) * subdim];
            for (accum, value) in sum.iter_mut().zip(slice) {
                *accum += *value;
            }
        }

        let mut next = centroids.clone();
        for centroid in 0..k {
            if counts[centroid] == 0 {
                continue;
            }
            let sum = &sums[centroid * subdim..(centroid + 1) * subdim];
            let target = &mut next[centroid * subdim..(centroid + 1) * subdim];
            for (slot, value) in target.iter_mut().zip(sum) {
                *slot = *value / counts[centroid] as f32;
            }
        }

        let max_drift = centroids
            .chunks_exact(subdim)
            .zip(next.chunks_exact(subdim))
            .map(|(left, right)| squared_l2(left, right).sqrt())
            .fold(0.0_f32, f32::max);
        centroids = next;
        if max_drift < LLOYD_DRIFT_TOL {
            break;
        }
    }
    centroids
}

fn kmeans_pp_init(
    rows: &[&[f32]],
    start: usize,
    subdim: usize,
    k: usize,
    rng: &mut fastrand::Rng,
) -> Vec<f32> {
    let mut centroids = Vec::with_capacity(k * subdim);
    let first = rng.usize(..rows.len());
    centroids.extend_from_slice(&rows[first][start..start + subdim]);

    while centroids.len() < k * subdim {
        let chosen = choose_next_centroid(rows, start, subdim, &centroids, rng);
        centroids.extend_from_slice(&rows[chosen][start..start + subdim]);
    }
    centroids
}

fn choose_next_centroid(
    rows: &[&[f32]],
    start: usize,
    subdim: usize,
    centroids: &[f32],
    rng: &mut fastrand::Rng,
) -> usize {
    let k_existing = centroids.len() / subdim;
    let distances = rows
        .iter()
        .map(|row| {
            let slice = &row[start..start + subdim];
            (0..k_existing)
                .map(|centroid| {
                    squared_l2(
                        slice,
                        &centroids[centroid * subdim..(centroid + 1) * subdim],
                    )
                })
                .fold(f32::INFINITY, f32::min)
        })
        .collect::<Vec<_>>();
    let total: f32 = distances.iter().sum();
    if total <= 0.0 {
        return 0;
    }
    let mut target = rng.f32() * total;
    for (index, distance) in distances.iter().copied().enumerate() {
        target -= distance;
        if target <= 0.0 {
            return index;
        }
    }
    rows.len() - 1
}

/// Return the nearest centroid index under squared L2.
pub(crate) fn nearest_centroid(row: &[f32], centroids: &[f32], k: usize, dim: usize) -> usize {
    let mut best = 0;
    let mut best_dist = f32::INFINITY;
    for centroid in 0..k {
        let center = &centroids[centroid * dim..(centroid + 1) * dim];
        let dist = squared_l2(row, center);
        if dist < best_dist {
            best = centroid;
            best_dist = dist;
        }
    }
    best
}

/// Compute squared Euclidean distance.
pub(crate) fn squared_l2(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(a, b)| {
            let delta = a - b;
            delta * delta
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kmeans_empty_cluster_policy_is_deterministic() {
        let rows = [
            vec![0.0, 0.0],
            vec![0.0, 0.0],
            vec![4.0, 4.0],
            vec![8.0, 8.0],
        ];
        let refs = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let mut left_rng = fastrand::Rng::with_seed(0xB66E_0001_u64);
        let mut right_rng = fastrand::Rng::with_seed(0xB66E_0001_u64);

        let left = kmeans_train_subspace(&refs, 0, 2, 4, &mut left_rng);
        let right = kmeans_train_subspace(&refs, 0, 2, 4, &mut right_rng);

        assert_eq!(left, right);
    }

    #[test]
    fn kmeans_two_separated_clusters_assigns_rows_by_cluster() {
        let rows = [
            vec![0.0, 0.0],
            vec![0.2, -0.1],
            vec![-0.2, 0.1],
            vec![10.0, 10.0],
            vec![10.2, 9.9],
            vec![9.8, 10.1],
        ];
        let refs = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let mut rng = fastrand::Rng::with_seed(0xB95E_0002_u64);

        let centroids = kmeans_train_subspace(&refs, 0, 2, 2, &mut rng);
        let assignments = refs
            .iter()
            .map(|row| nearest_centroid(row, &centroids, 2, 2))
            .collect::<Vec<_>>();

        let left = assignments[0];
        let right = assignments[3];
        assert_ne!(left, right, "separated clusters must not collapse");
        assert!(
            assignments[..3].iter().all(|&cluster| cluster == left),
            "left cluster rows should share one assignment: {assignments:?}"
        );
        assert!(
            assignments[3..].iter().all(|&cluster| cluster == right),
            "right cluster rows should share one assignment: {assignments:?}"
        );
    }
}
