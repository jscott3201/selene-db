//! Portable distance kernels used by the HNSW search layer.

use crate::DistanceMetric;

const UNIT_LENGTH_TOLERANCE: f32 = 1e-5;

/// Compute the dot product of two vectors.
///
/// The scalar path uses an 8-wide accumulator to reduce long-vector
/// accumulation error without requiring platform-specific intrinsics.
///
/// # Panics
///
/// Panics in debug builds when the slices have different lengths. Release
/// builds keep memory safety but the result is not meaningful for mismatched
/// dimensions.
#[must_use]
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "dot product dimension mismatch");
    #[cfg(feature = "simd-simsimd")]
    {
        if let Some(value) = <f32 as simsimd::SpatialSimilarity>::dot(a, b) {
            return value as f32;
        }
    }
    dot_product_scalar(a, b)
}

/// Compute squared Euclidean distance.
///
/// Callers that need the true Euclidean metric can take the square root; HNSW
/// construction and search often compare squared distances directly.
///
/// # Panics
///
/// Panics in debug builds when the slices have different lengths. Release
/// builds keep memory safety but the result is not meaningful for mismatched
/// dimensions.
#[must_use]
pub fn l2_squared(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "L2 dimension mismatch");
    #[cfg(feature = "simd-simsimd")]
    {
        if let Some(value) = <f32 as simsimd::SpatialSimilarity>::sqeuclidean(a, b) {
            return value as f32;
        }
    }
    l2_squared_scalar(a, b)
}

/// Compute cosine similarity.
///
/// Returns `0.0` when either vector has zero magnitude. This preserves the
/// donor convention and avoids simsimd's cosine-distance zero-vector behavior.
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "cosine dimension mismatch");
    let dot = dot_product(a, b);
    let mag_a = dot_product(a, a).sqrt();
    let mag_b = dot_product(b, b).sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        0.0
    } else {
        dot / (mag_a * mag_b)
    }
}

/// Return true when the squared norm is within `1e-5` of `1.0`.
///
/// This intentionally checks the squared norm to avoid an unnecessary square
/// root in callers that only need the unit-length predicate.
#[must_use]
pub fn is_unit_length(v: &[f32]) -> bool {
    (dot_product(v, v) - 1.0).abs() <= UNIT_LENGTH_TOLERANCE
}

/// Compute the HNSW ordering distance for `metric`.
///
/// Smaller values are closer. Cosine distance is `1.0 - cosine_similarity`,
/// L2 is the Euclidean distance, and dot-product distance is negative
/// similarity so larger dot products sort first.
#[must_use]
pub fn distance(metric: DistanceMetric, a: &[f32], b: &[f32]) -> f32 {
    match metric {
        DistanceMetric::Cosine => 1.0 - cosine_similarity(a, b),
        DistanceMetric::L2 => l2_squared(a, b).sqrt(),
        DistanceMetric::Dot => -dot_product(a, b),
    }
}

fn dot_product_scalar(a: &[f32], b: &[f32]) -> f32 {
    let mut sum = [0.0f32; 8];
    let chunks_a = a.chunks_exact(8);
    let chunks_b = b.chunks_exact(8);
    let rem_a = chunks_a.remainder();
    let rem_b = chunks_b.remainder();

    for (a8, b8) in chunks_a.zip(chunks_b) {
        sum[0] += a8[0] * b8[0];
        sum[1] += a8[1] * b8[1];
        sum[2] += a8[2] * b8[2];
        sum[3] += a8[3] * b8[3];
        sum[4] += a8[4] * b8[4];
        sum[5] += a8[5] * b8[5];
        sum[6] += a8[6] * b8[6];
        sum[7] += a8[7] * b8[7];
    }

    let total = (sum[0] + sum[1]) + (sum[2] + sum[3]) + (sum[4] + sum[5]) + (sum[6] + sum[7]);
    rem_a
        .iter()
        .zip(rem_b)
        .fold(total, |acc, (x, y)| acc + (x * y))
}

fn l2_squared_scalar(a: &[f32], b: &[f32]) -> f32 {
    let mut sum = [0.0f32; 8];
    let chunks_a = a.chunks_exact(8);
    let chunks_b = b.chunks_exact(8);
    let rem_a = chunks_a.remainder();
    let rem_b = chunks_b.remainder();

    for (a8, b8) in chunks_a.zip(chunks_b) {
        sum[0] += square(a8[0] - b8[0]);
        sum[1] += square(a8[1] - b8[1]);
        sum[2] += square(a8[2] - b8[2]);
        sum[3] += square(a8[3] - b8[3]);
        sum[4] += square(a8[4] - b8[4]);
        sum[5] += square(a8[5] - b8[5]);
        sum[6] += square(a8[6] - b8[6]);
        sum[7] += square(a8[7] - b8[7]);
    }

    let total = (sum[0] + sum[1]) + (sum[2] + sum[3]) + (sum[4] + sum[5]) + (sum[6] + sum[7]);
    rem_a
        .iter()
        .zip(rem_b)
        .fold(total, |acc, (x, y)| acc + square(x - y))
}

fn square(value: f32) -> f32 {
    value * value
}
