//! Native dense-vector metric kernels and exact-search helpers.
//!
//! The ANN index layer builds on these primitives so approximate indexes and
//! exhaustive recall oracles share one definition of distance, tie-breaking,
//! and vector validity.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use serde::{Deserialize, Serialize};

use crate::{CoreError, CoreResult, VectorValue};

/// Distance metric for native dense vectors.
///
/// All metrics return a score where **lower is better**. `NegativeInnerProduct`
/// is the max-inner-product-search adapter: vectors with larger dot products
/// produce smaller, more favorable scores.
#[derive(
    Clone,
    Copy,
    Debug,
    Deserialize,
    Eq,
    Hash,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub enum VectorMetric {
    /// Squared Euclidean distance (`sum((lhs_i - rhs_i)^2)`).
    SquaredEuclidean,
    /// Cosine distance (`1 - cosine_similarity`).
    Cosine,
    /// Negated dot product (`-sum(lhs_i * rhs_i)`), lower-is-better MIPS.
    NegativeInnerProduct,
}

impl VectorMetric {
    /// Compute this metric for two vectors.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::VectorDimensionMismatch`] if dimensions differ.
    /// [`VectorMetric::Cosine`] also returns [`CoreError::VectorZeroNorm`]
    /// when either vector has zero magnitude.
    pub fn distance(self, lhs: &VectorValue, rhs: &VectorValue) -> CoreResult<f64> {
        let lhs = lhs.as_slice();
        let rhs = rhs.as_slice();
        check_same_dimension(lhs.len(), rhs.len())?;
        Ok(canonical_score(match self {
            Self::SquaredEuclidean => squared_euclidean(lhs, rhs),
            Self::Cosine => cosine_distance(lhs, rhs)?,
            Self::NegativeInnerProduct => -dot(lhs, rhs),
        }))
    }
}

/// A single exact vector-search result.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorSearchHit<K> {
    /// Caller-owned candidate key, such as a node id or row ordinal.
    pub key: K,
    /// Lower-is-better score under the requested [`VectorMetric`].
    pub distance: f64,
}

/// Return the exact top-`k` nearest vector candidates.
///
/// This is intentionally a small exhaustive oracle, not an ANN index. Future
/// HNSW/IVF/PQ implementations should use it for recall validation and for
/// small result sets where index build cost cannot amortize.
///
/// Ties are deterministic: lower distance wins, then lower `key` wins.
///
/// # Errors
///
/// Returns a vector metric error if any candidate cannot be compared to
/// `query` under `metric`.
pub fn exact_vector_top_k<'a, K, I>(
    metric: VectorMetric,
    query: &VectorValue,
    candidates: I,
    k: usize,
) -> CoreResult<Vec<VectorSearchHit<K>>>
where
    K: Ord,
    I: IntoIterator<Item = (K, &'a VectorValue)>,
{
    if k == 0 {
        return Ok(Vec::new());
    }

    let mut heap = BinaryHeap::new();
    for (key, vector) in candidates {
        let distance = metric.distance(query, vector)?;
        let entry = HeapEntry { distance, key };
        if heap.len() < k {
            heap.push(entry);
            continue;
        }
        let Some(worst) = heap.peek() else {
            continue;
        };
        if entry.cmp(worst).is_lt() {
            heap.pop();
            heap.push(entry);
        }
    }

    let mut hits: Vec<_> = heap
        .into_iter()
        .map(|entry| VectorSearchHit {
            key: entry.key,
            distance: entry.distance,
        })
        .collect();
    hits.sort_by(compare_hit);
    Ok(hits)
}

#[derive(Debug)]
struct HeapEntry<K> {
    distance: f64,
    key: K,
}

impl<K: Eq> Eq for HeapEntry<K> {}

impl<K: Eq> PartialEq for HeapEntry<K> {
    fn eq(&self, rhs: &Self) -> bool {
        self.distance.to_bits() == rhs.distance.to_bits() && self.key == rhs.key
    }
}

impl<K: Ord> Ord for HeapEntry<K> {
    fn cmp(&self, rhs: &Self) -> Ordering {
        self.distance
            .total_cmp(&rhs.distance)
            .then_with(|| self.key.cmp(&rhs.key))
    }
}

impl<K: Ord> PartialOrd for HeapEntry<K> {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.cmp(rhs))
    }
}

fn compare_hit<K: Ord>(lhs: &VectorSearchHit<K>, rhs: &VectorSearchHit<K>) -> Ordering {
    lhs.distance
        .total_cmp(&rhs.distance)
        .then_with(|| lhs.key.cmp(&rhs.key))
}

fn check_same_dimension(lhs: usize, rhs: usize) -> CoreResult<()> {
    if lhs == rhs {
        Ok(())
    } else {
        Err(CoreError::VectorDimensionMismatch { lhs, rhs })
    }
}

fn squared_euclidean(lhs: &[f32], rhs: &[f32]) -> f64 {
    lhs.iter()
        .zip(rhs)
        .map(|(&lhs, &rhs)| {
            let delta = f64::from(lhs) - f64::from(rhs);
            delta * delta
        })
        .sum()
}

fn cosine_distance(lhs: &[f32], rhs: &[f32]) -> CoreResult<f64> {
    let lhs_norm = dot(lhs, lhs);
    if lhs_norm == 0.0 {
        return Err(CoreError::VectorZeroNorm { side: "lhs" });
    }
    let rhs_norm = dot(rhs, rhs);
    if rhs_norm == 0.0 {
        return Err(CoreError::VectorZeroNorm { side: "rhs" });
    }
    let similarity = dot(lhs, rhs) / (lhs_norm.sqrt() * rhs_norm.sqrt());
    Ok(1.0 - similarity.clamp(-1.0, 1.0))
}

fn dot(lhs: &[f32], rhs: &[f32]) -> f64 {
    lhs.iter()
        .zip(rhs)
        .map(|(&lhs, &rhs)| f64::from(lhs) * f64::from(rhs))
        .sum()
}

fn canonical_score(score: f64) -> f64 {
    if score == 0.0 { 0.0 } else { score }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector(components: &[f32]) -> VectorValue {
        VectorValue::new(components.to_vec()).expect("test vector is valid")
    }

    #[test]
    fn squared_euclidean_uses_f64_accumulation() {
        let lhs = vector(&[1.0, 2.0, 3.0]);
        let rhs = vector(&[1.0, 4.0, -1.0]);
        let distance = VectorMetric::SquaredEuclidean
            .distance(&lhs, &rhs)
            .expect("dimensions match");
        assert_eq!(distance, 20.0);
    }

    #[test]
    fn negative_inner_product_is_lower_for_larger_dot_product() {
        let query = vector(&[1.0, 2.0]);
        let low_dot = vector(&[1.0, 0.0]);
        let high_dot = vector(&[2.0, 2.0]);

        let low_score = VectorMetric::NegativeInnerProduct
            .distance(&query, &low_dot)
            .expect("dimensions match");
        let high_score = VectorMetric::NegativeInnerProduct
            .distance(&query, &high_dot)
            .expect("dimensions match");

        assert!(high_score < low_score);
        assert_eq!(low_score, -1.0);
        assert_eq!(high_score, -6.0);
    }

    #[test]
    fn metric_distance_canonicalizes_signed_zero_scores() {
        let lhs = vector(&[0.0, -0.0]);
        let rhs = vector(&[1.0, -1.0]);

        let distance = VectorMetric::NegativeInnerProduct
            .distance(&lhs, &rhs)
            .expect("dimensions match");

        assert_eq!(distance.to_bits(), 0.0_f64.to_bits());
    }

    #[test]
    fn cosine_distance_handles_identical_and_opposite_vectors() {
        let lhs = vector(&[1.0, 0.0]);
        let same = vector(&[2.0, 0.0]);
        let opposite = vector(&[-1.0, 0.0]);

        assert_eq!(VectorMetric::Cosine.distance(&lhs, &same).unwrap(), 0.0);
        assert_eq!(VectorMetric::Cosine.distance(&lhs, &opposite).unwrap(), 2.0);
    }

    #[test]
    fn cosine_rejects_zero_norm_vectors() {
        let zero = vector(&[0.0, 0.0]);
        let rhs = vector(&[1.0, 0.0]);

        let error = VectorMetric::Cosine.distance(&zero, &rhs).unwrap_err();
        assert!(matches!(error, CoreError::VectorZeroNorm { side: "lhs" }));

        let error = VectorMetric::Cosine.distance(&rhs, &zero).unwrap_err();
        assert!(matches!(error, CoreError::VectorZeroNorm { side: "rhs" }));
    }

    #[test]
    fn distance_rejects_dimension_mismatch() {
        let lhs = vector(&[1.0, 2.0]);
        let rhs = vector(&[1.0, 2.0, 3.0]);

        let error = VectorMetric::SquaredEuclidean
            .distance(&lhs, &rhs)
            .unwrap_err();
        assert!(matches!(
            error,
            CoreError::VectorDimensionMismatch { lhs: 2, rhs: 3 }
        ));
    }

    #[test]
    fn exact_top_k_returns_empty_for_zero_k() {
        let query = vector(&[0.0]);
        let candidate = vector(&[1.0]);
        let candidates = [(7_u64, &candidate)];

        let hits = exact_vector_top_k(VectorMetric::SquaredEuclidean, &query, candidates, 0)
            .expect("zero k does not inspect candidates");

        assert!(hits.is_empty());
    }

    #[test]
    fn exact_top_k_is_distance_then_key_ordered() {
        let query = vector(&[0.0]);
        let one = vector(&[1.0]);
        let two = vector(&[2.0]);
        let candidates = [(3_u64, &two), (2, &one), (1, &one)];

        let hits = exact_vector_top_k(VectorMetric::SquaredEuclidean, &query, candidates, 2)
            .expect("all dimensions match");

        assert_eq!(
            hits,
            vec![
                VectorSearchHit {
                    key: 1,
                    distance: 1.0
                },
                VectorSearchHit {
                    key: 2,
                    distance: 1.0
                }
            ]
        );
    }

    #[test]
    fn exact_top_k_surfaces_candidate_metric_errors() {
        let query = vector(&[0.0]);
        let candidate = vector(&[1.0, 2.0]);
        let candidates = [(1_u64, &candidate)];

        let error =
            exact_vector_top_k(VectorMetric::SquaredEuclidean, &query, candidates, 10).unwrap_err();

        assert!(matches!(
            error,
            CoreError::VectorDimensionMismatch { lhs: 1, rhs: 2 }
        ));
    }
}
