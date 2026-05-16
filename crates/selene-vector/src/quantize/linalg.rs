//! Deterministic linalg helpers used by OPQ training.

use crate::VectorError;

const JACOBI_SWEEPS: usize = 50;
const JACOBI_TOL: f64 = 1.0e-10;

pub(crate) fn identity(dim: usize) -> Vec<f32> {
    let mut out = vec![0.0; dim * dim];
    for idx in 0..dim {
        out[idx * dim + idx] = 1.0;
    }
    out
}

pub(crate) fn mat_vec(matrix: &[f32], vector: &[f32], dim: usize) -> Vec<f32> {
    let mut out = vec![0.0; dim];
    for (row, value) in out.iter_mut().enumerate().take(dim) {
        let start = row * dim;
        *value = matrix[start..start + dim]
            .iter()
            .zip(vector)
            .map(|(left, right)| left * right)
            .sum();
    }
    out
}

pub(crate) fn mat_transpose_vec(matrix: &[f32], vector: &[f32], dim: usize) -> Vec<f32> {
    let mut out = vec![0.0; dim];
    for (col, value) in out.iter_mut().enumerate().take(dim) {
        let mut sum = 0.0;
        for row in 0..dim {
            sum += matrix[row * dim + col] * vector[row];
        }
        *value = sum;
    }
    out
}

pub(crate) fn rotation_drift(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| (left - right).abs())
        .fold(0.0_f32, f32::max)
}

pub(crate) fn is_orthonormal(matrix: &[f32], dim: usize, tolerance: f32) -> bool {
    if matrix.len() != dim.saturating_mul(dim) {
        return false;
    }
    for row in 0..dim {
        for other in row..dim {
            let mut dot = 0.0;
            for coord in 0..dim {
                dot += matrix[row * dim + coord] * matrix[other * dim + coord];
            }
            let expected = if row == other { 1.0 } else { 0.0 };
            if (dot - expected).abs() > tolerance {
                return false;
            }
        }
    }
    true
}

pub(crate) fn is_proper_rotation(matrix: &[f32], dim: usize, tolerance: f32) -> bool {
    is_orthonormal(matrix, dim, tolerance)
        && determinant(matrix, dim).is_some_and(|det| (det - 1.0).abs() <= f64::from(tolerance))
}

pub(crate) fn determinant(matrix: &[f32], dim: usize) -> Option<f64> {
    if matrix.len() != dim.checked_mul(dim)? {
        return None;
    }
    let mut work = matrix
        .iter()
        .map(|value| f64::from(*value))
        .collect::<Vec<_>>();
    let mut det = 1.0_f64;
    let mut sign = 1.0_f64;
    for pivot_col in 0..dim {
        let pivot_row = (pivot_col..dim).max_by(|left, right| {
            work[*left * dim + pivot_col]
                .abs()
                .total_cmp(&work[*right * dim + pivot_col].abs())
        })?;
        let pivot = work[pivot_row * dim + pivot_col];
        if pivot.abs() <= f64::EPSILON {
            return Some(0.0);
        }
        if pivot_row != pivot_col {
            for col in 0..dim {
                work.swap(pivot_row * dim + col, pivot_col * dim + col);
            }
            sign = -sign;
        }
        let pivot = work[pivot_col * dim + pivot_col];
        det *= pivot;
        for row in pivot_col + 1..dim {
            let factor = work[row * dim + pivot_col] / pivot;
            for col in pivot_col + 1..dim {
                work[row * dim + col] -= factor * work[pivot_col * dim + col];
            }
        }
    }
    Some(sign * det)
}

/// Return the closest proper rotation for the square cross-covariance matrix.
///
/// The SVD path first computes `U * V^T`. If that orthonormal transform is a
/// reflection (`det=-1`), the last column of `U` is flipped before recomputing
/// the Procrustes solution, yielding `det=+1`. Near-zero determinants are
/// rejected as degenerate numerical failures instead of making an unstable
/// sign choice.
pub(crate) fn procrustes_rotation(
    cross: &[f32],
    dim: usize,
    context: &'static str,
) -> Result<Vec<f32>, VectorError> {
    let mut svd = svd_square(cross, dim, context)?;
    let mut rotation = multiply_u_v_transpose(&svd.u, &svd.v, dim);
    let det = determinant(&rotation, dim).ok_or_else(|| VectorError::OpqTrainingFailed {
        context,
        reason: "Procrustes rotation determinant shape mismatch".into(),
    })?;
    if det.abs() < 1.0e-10 {
        return Err(VectorError::OpqTrainingFailed {
            context,
            reason: "Procrustes rotation determinant is numerically degenerate".into(),
        });
    }
    if det < 0.0 {
        let last_col = dim.saturating_sub(1);
        for row in 0..dim {
            svd.u[row * dim + last_col] *= -1.0;
        }
        rotation = multiply_u_v_transpose(&svd.u, &svd.v, dim);
    }
    if !is_proper_rotation(&rotation, dim, 1.0e-4) {
        return Err(VectorError::OpqTrainingFailed {
            context,
            reason: "Procrustes rotation is not a proper rotation".into(),
        });
    }
    Ok(rotation)
}

fn multiply_u_v_transpose(u: &[f32], v: &[f32], dim: usize) -> Vec<f32> {
    let mut rotation = vec![0.0; dim * dim];
    for row in 0..dim {
        for col in 0..dim {
            let mut sum = 0.0;
            for component in 0..dim {
                sum += u[row * dim + component] * v[col * dim + component];
            }
            rotation[row * dim + col] = sum;
        }
    }
    rotation
}

struct Svd {
    u: Vec<f32>,
    v: Vec<f32>,
}

fn svd_square(a: &[f32], dim: usize, context: &'static str) -> Result<Svd, VectorError> {
    if a.len() != dim.saturating_mul(dim) {
        return Err(VectorError::OpqTrainingFailed {
            context,
            reason: "SVD input shape mismatch".into(),
        });
    }
    let ata = ata(a, dim);
    let (eigenvalues, v64) = jacobi_eigen_symmetric(&ata, dim);
    let mut order = (0..dim).collect::<Vec<_>>();
    order.sort_by(|left, right| eigenvalues[*right].total_cmp(&eigenvalues[*left]));

    let mut v = vec![0.0_f64; dim * dim];
    let mut singulars = vec![0.0_f64; dim];
    for (out_col, in_col) in order.iter().copied().enumerate() {
        singulars[out_col] = eigenvalues[in_col].max(0.0).sqrt();
        for row in 0..dim {
            v[row * dim + out_col] = v64[row * dim + in_col];
        }
    }

    let mut u = vec![0.0_f64; dim * dim];
    for col in 0..dim {
        if singulars[col] > 1.0e-8 {
            for row in 0..dim {
                let mut sum = 0.0;
                for inner in 0..dim {
                    sum += f64::from(a[row * dim + inner]) * v[inner * dim + col];
                }
                u[row * dim + col] = sum / singulars[col];
            }
        } else {
            u[col * dim + col] = 1.0;
        }
    }
    orthonormalize_columns(&mut u, dim);
    orthonormalize_columns(&mut v, dim);

    Ok(Svd {
        u: u.into_iter().map(|value| value as f32).collect(),
        v: v.into_iter().map(|value| value as f32).collect(),
    })
}

fn ata(a: &[f32], dim: usize) -> Vec<f64> {
    let mut out = vec![0.0; dim * dim];
    for row in 0..dim {
        for col in row..dim {
            let mut sum = 0.0;
            for idx in 0..dim {
                sum += f64::from(a[idx * dim + row]) * f64::from(a[idx * dim + col]);
            }
            out[row * dim + col] = sum;
            out[col * dim + row] = sum;
        }
    }
    out
}

fn jacobi_eigen_symmetric(input: &[f64], dim: usize) -> (Vec<f64>, Vec<f64>) {
    let mut a = input.to_vec();
    let mut v = vec![0.0; dim * dim];
    for idx in 0..dim {
        v[idx * dim + idx] = 1.0;
    }
    for _ in 0..JACOBI_SWEEPS {
        let Some((p, q, max_offdiag)) = largest_offdiag(&a, dim) else {
            break;
        };
        if max_offdiag < JACOBI_TOL {
            break;
        }
        let app = a[p * dim + p];
        let aqq = a[q * dim + q];
        let apq = a[p * dim + q];
        let tau = (aqq - app) / (2.0 * apq);
        let t = tau.signum() / (tau.abs() + (1.0 + tau * tau).sqrt());
        let c = 1.0 / (1.0 + t * t).sqrt();
        let s = t * c;

        for k in 0..dim {
            if k != p && k != q {
                let akp = a[k * dim + p];
                let akq = a[k * dim + q];
                let next_kp = c * akp - s * akq;
                let next_kq = s * akp + c * akq;
                a[k * dim + p] = next_kp;
                a[p * dim + k] = next_kp;
                a[k * dim + q] = next_kq;
                a[q * dim + k] = next_kq;
            }
        }
        let next_pp = c * c * app - 2.0 * s * c * apq + s * s * aqq;
        let next_qq = s * s * app + 2.0 * s * c * apq + c * c * aqq;
        a[p * dim + p] = next_pp;
        a[q * dim + q] = next_qq;
        a[p * dim + q] = 0.0;
        a[q * dim + p] = 0.0;

        for row in 0..dim {
            let vkp = v[row * dim + p];
            let vkq = v[row * dim + q];
            v[row * dim + p] = c * vkp - s * vkq;
            v[row * dim + q] = s * vkp + c * vkq;
        }
    }
    let eigenvalues = (0..dim).map(|idx| a[idx * dim + idx]).collect();
    (eigenvalues, v)
}

fn largest_offdiag(a: &[f64], dim: usize) -> Option<(usize, usize, f64)> {
    let mut best = None;
    let mut best_value = 0.0;
    for row in 0..dim {
        for col in row + 1..dim {
            let value = a[row * dim + col].abs();
            if value > best_value {
                best = Some((row, col, value));
                best_value = value;
            }
        }
    }
    best
}

fn orthonormalize_columns(matrix: &mut [f64], dim: usize) {
    for col in 0..dim {
        for prev in 0..col {
            let projection = column_dot(matrix, dim, col, prev);
            for row in 0..dim {
                matrix[row * dim + col] -= projection * matrix[row * dim + prev];
            }
        }
        let norm = column_norm(matrix, dim, col);
        if norm > 1.0e-10 {
            for row in 0..dim {
                matrix[row * dim + col] /= norm;
            }
            continue;
        }
        fill_basis_column(matrix, dim, col);
    }
}

fn fill_basis_column(matrix: &mut [f64], dim: usize, col: usize) {
    for row in 0..dim {
        matrix[row * dim + col] = 0.0;
    }
    let basis = col.min(dim.saturating_sub(1));
    matrix[basis * dim + col] = 1.0;
    for prev in 0..col {
        let projection = column_dot(matrix, dim, col, prev);
        for row in 0..dim {
            matrix[row * dim + col] -= projection * matrix[row * dim + prev];
        }
    }
    let norm = column_norm(matrix, dim, col);
    if norm > 1.0e-10 {
        for row in 0..dim {
            matrix[row * dim + col] /= norm;
        }
    }
}

fn column_dot(matrix: &[f64], dim: usize, left: usize, right: usize) -> f64 {
    (0..dim)
        .map(|row| matrix[row * dim + left] * matrix[row * dim + right])
        .sum()
}

fn column_norm(matrix: &[f64], dim: usize, col: usize) -> f64 {
    column_dot(matrix, dim, col, col).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procrustes_identity_is_orthonormal() {
        let rotation = procrustes_rotation(&identity(4), 4, "test").unwrap();

        assert!(is_orthonormal(&rotation, 4, 1.0e-5));
        assert!(is_proper_rotation(&rotation, 4, 1.0e-5));
    }

    #[test]
    fn procrustes_step_aligns_x_to_y_not_y_to_x() {
        let true_rotation = vec![0.0, -1.0, 1.0, 0.0];
        let samples = [[2.0, 0.0], [0.0, 1.0], [3.0, 1.0], [-1.0, 2.0]];
        let targets = samples
            .iter()
            .map(|sample| mat_vec(&true_rotation, sample, 2))
            .collect::<Vec<_>>();
        let mut cross = vec![0.0; 4];
        for (sample, target) in samples.iter().zip(&targets) {
            for row in 0..2 {
                for col in 0..2 {
                    cross[row * 2 + col] += target[row] * sample[col];
                }
            }
        }

        let rotation = procrustes_rotation(&cross, 2, "test").unwrap();

        for (observed, expected) in rotation.iter().zip(&true_rotation) {
            assert!(
                (observed - expected).abs() <= 1.0e-4,
                "observed rotation {rotation:?} != {true_rotation:?}"
            );
        }
    }

    #[test]
    fn procrustes_reflection_input_returns_closest_proper_rotation() {
        let cross = vec![1.0, 0.0, 0.0, -1.0];

        let rotation = procrustes_rotation(&cross, 2, "test").unwrap();

        assert!(is_proper_rotation(&rotation, 2, 1.0e-5));
        assert!(determinant(&rotation, 2).unwrap() > 0.0);
        assert_eq!(rotation, identity(2));
    }

    #[test]
    fn svd_square_two_by_two_known_singular_values() {
        let input = [3.0, 0.0, 0.0, 2.0];

        let svd = svd_square(&input, 2, "test").unwrap();

        assert_eq!(svd.u, identity(2));
        assert_eq!(svd.v, identity(2));
        let singular_0 = svd.u[0] * input[0] * svd.v[0] + svd.u[2] * input[3] * svd.v[2];
        let singular_1 = svd.u[1] * input[0] * svd.v[1] + svd.u[3] * input[3] * svd.v[3];
        assert!((singular_0 - 3.0).abs() <= 1.0e-6);
        assert!((singular_1 - 2.0).abs() <= 1.0e-6);
    }

    #[test]
    fn jacobi_eigen_2x2_known_eigenpairs() {
        let input = [2.0, 1.0, 1.0, 2.0];

        let (values, vectors) = jacobi_eigen_symmetric(&input, 2);

        let high = values
            .iter()
            .position(|value| (*value - 3.0).abs() <= 1.0e-8)
            .expect("eigenvalue 3 present");
        let low = values
            .iter()
            .position(|value| (*value - 1.0).abs() <= 1.0e-8)
            .expect("eigenvalue 1 present");
        let inv_sqrt_2 = std::f64::consts::FRAC_1_SQRT_2;
        let high_dot = (vectors[high] * inv_sqrt_2 + vectors[2 + high] * inv_sqrt_2).abs();
        let low_dot = (vectors[low] * inv_sqrt_2 - vectors[2 + low] * inv_sqrt_2).abs();
        assert!((high_dot - 1.0).abs() <= 1.0e-8);
        assert!((low_dot - 1.0).abs() <= 1.0e-8);
    }

    #[test]
    fn mat_vec_non_identity_hand_derived() {
        let matrix = [1.0, 2.0, 3.0, 4.0];
        let vector = [1.0, 1.0];

        assert_eq!(mat_vec(&matrix, &vector, 2), vec![3.0, 7.0]);
    }

    #[test]
    fn mat_transpose_vec_non_symmetric() {
        let matrix = [1.0, 2.0, 3.0, 4.0];
        let vector = [1.0, 1.0];

        assert_eq!(mat_transpose_vec(&matrix, &vector, 2), vec![4.0, 6.0]);
    }

    #[test]
    fn mat_transpose_vec_reverses_identity_rotation() {
        let matrix = identity(3);
        let vector = [1.0, 2.0, 3.0];

        assert_eq!(mat_transpose_vec(&matrix, &vector, 3), vector);
    }
}
