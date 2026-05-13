//! OPQ rotation training.

use crate::clustering::squared_l2;
use crate::{PqParams, VectorError};

use super::linalg;
use super::pq::PqCodebook;

const OPQ_OUTER_ITERS: usize = 25;
const ROTATION_DRIFT_TOL: f32 = 1.0e-5;
const ERROR_DELTA_TOL: f32 = 1.0e-6;

pub(crate) fn train(
    dim: usize,
    params: PqParams,
    rows: &[&[f32]],
    seed: u64,
    context: &'static str,
) -> Result<PqCodebook, VectorError> {
    if params.m_subspaces == 1 {
        return PqCodebook::train_plain(dim, params, rows, seed, context, None);
    }

    let plain_params = PqParams {
        use_opq: false,
        use_polysemous: false,
        hamming_threshold_ratio: 0.5,
        ..params
    };
    let plain = PqCodebook::train_plain(dim, plain_params, rows, seed, context, None)?;
    let mut best = plain.clone();
    let mut best_error = reconstruction_mse(&plain, rows, dim);
    let identity = PqCodebook::train_plain(
        dim,
        plain_params,
        rows,
        seed,
        context,
        Some(linalg::identity(dim)),
    )?;
    let identity_error = reconstruction_mse(&identity, rows, dim);
    if identity_error + ERROR_DELTA_TOL < best_error {
        best = identity;
        best_error = identity_error;
    }

    let mut rotation = linalg::identity(dim);
    let mut previous_error = f32::INFINITY;
    for _ in 0..OPQ_OUTER_ITERS {
        let rotated = rotate_rows(rows, &rotation, dim);
        let rotated_refs = rotated.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let current =
            PqCodebook::train_plain(dim, plain_params, &rotated_refs, seed, context, None)?;
        let decoded = reconstruct_rows(&current, &rotated_refs, dim);
        let cross = cross_covariance(rows, &decoded, dim);
        let next_rotation = linalg::procrustes_rotation(&cross, dim, context)?;
        let next_rotated = rotate_rows(rows, &next_rotation, dim);
        let next_refs = next_rotated.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let candidate = PqCodebook::train_plain(
            dim,
            plain_params,
            &next_refs,
            seed,
            context,
            Some(next_rotation.clone()),
        )?;
        let candidate_error = reconstruction_mse(&candidate, rows, dim);
        if candidate_error + ERROR_DELTA_TOL < best_error {
            best_error = candidate_error;
            best = candidate;
        }
        let drift = linalg::rotation_drift(&rotation, &next_rotation);
        let delta = (previous_error - candidate_error).abs();
        rotation = next_rotation;
        previous_error = candidate_error;
        if drift < ROTATION_DRIFT_TOL || delta < ERROR_DELTA_TOL {
            break;
        }
    }

    if best
        .rotation
        .as_deref()
        .is_some_and(|rotation| !linalg::is_orthonormal(rotation, dim, 1.0e-5))
    {
        return Err(VectorError::OpqTrainingFailed {
            context,
            reason: "OPQ training did not produce an orthonormal rotation".into(),
        });
    }
    Ok(best)
}

fn rotate_rows(rows: &[&[f32]], rotation: &[f32], dim: usize) -> Vec<Vec<f32>> {
    rows.iter()
        .map(|row| linalg::mat_vec(rotation, row, dim))
        .collect()
}

fn reconstruct_rows(codebook: &PqCodebook, rows: &[&[f32]], dim: usize) -> Vec<Vec<f32>> {
    rows.iter()
        .map(|row| {
            let mut codes = Vec::with_capacity(codebook.m_subspaces as usize);
            codebook.encode_row(row, &mut codes);
            let mut decoded = vec![0.0; dim];
            codebook
                .decode_codes(&codes, &mut decoded)
                .expect("OPQ reconstruction codes match codebook");
            decoded
        })
        .collect()
}

fn reconstruction_mse(codebook: &PqCodebook, rows: &[&[f32]], dim: usize) -> f32 {
    let mut total = 0.0;
    for row in rows {
        let mut codes = Vec::with_capacity(codebook.m_subspaces as usize);
        codebook.encode_row(row, &mut codes);
        let mut decoded = vec![0.0; dim];
        codebook
            .decode_codes(&codes, &mut decoded)
            .expect("OPQ reconstruction codes match codebook");
        total += squared_l2(row, &decoded);
    }
    total / rows.len().max(1) as f32
}

fn cross_covariance(rows: &[&[f32]], decoded_rotated: &[Vec<f32>], dim: usize) -> Vec<f32> {
    let mut out = vec![0.0; dim * dim];
    for (row, decoded) in rows.iter().zip(decoded_rotated) {
        for left in 0..dim {
            for right in 0..dim {
                out[left * dim + right] += row[left] * decoded[right];
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(count: usize, dim: usize) -> Vec<Vec<f32>> {
        (0..count)
            .map(|row| {
                (0..dim)
                    .map(|coord| ((row as f32 * 0.031) + (coord as f32 * 0.17)).sin())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn opq_returns_plain_codebook_when_outer_loop_cannot_beat_baseline() {
        let rows = rows(256, 4);
        let refs = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let params = PqParams {
            m_subspaces: 2,
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq: true,
            use_polysemous: false,
            hamming_threshold_ratio: 0.5,
        };
        let plain_params = PqParams {
            use_opq: false,
            use_polysemous: false,
            hamming_threshold_ratio: 0.5,
            ..params
        };
        let plain =
            PqCodebook::train_plain(4, plain_params, &refs, 0xB66E_0001_u64, "test", None).unwrap();

        let trained = train(4, params, &refs, 0xB66E_0001_u64, "test").unwrap();

        assert!(trained.rotation.is_none());
        assert!(
            (reconstruction_mse(&trained, &refs, 4) - reconstruction_mse(&plain, &refs, 4)).abs()
                <= 1.0e-6
        );
    }
}
