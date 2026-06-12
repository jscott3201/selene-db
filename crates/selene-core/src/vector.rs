//! Native dense-vector metric kernels and exact-search helpers.
//!
//! The ANN index layer builds on these primitives so approximate indexes and
//! exhaustive recall oracles share one definition of distance, tie-breaking,
//! and vector validity.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use serde::{Deserialize, Serialize};

use crate::{CoreError, CoreResult, VectorValue};

mod kernels;
mod turbo_quant;

use kernels::{
    cosine_distance, cosine_distance_with_lhs_norm, cosine_distance_with_norms, dot,
    squared_euclidean, validate_precomputed_squared_norm,
};
pub use turbo_quant::{
    TURBO_QUANT_BLOCK_ROWS, TurboQuantBitWidth, TurboQuantBlockedCodes, TurboQuantCodebook,
    TurboQuantCodebookKind, TurboQuantCodecError, TurboQuantCodecResult, TurboQuantPackedCodes,
};

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
    /// Bind this metric to one query vector for repeated candidate scoring.
    ///
    /// This precomputes metric-specific query state, such as cosine query norm,
    /// so exact scans and ANN traversals do not redo invariant work for every
    /// candidate.
    ///
    /// # Errors
    ///
    /// [`VectorMetric::Cosine`] returns [`CoreError::VectorZeroNorm`] when the
    /// query has zero magnitude.
    pub fn bind_query(self, query: &VectorValue) -> CoreResult<VectorMetricQuery<'_>> {
        VectorMetricQuery::new(self, query)
    }

    /// Bind this metric to one query vector with a precomputed query squared norm.
    ///
    /// When `query_squared_norm` is the query's actual squared norm, this is
    /// equivalent to [`Self::bind_query`]. It lets ANN indexes cache entry
    /// norms for cosine centroid assignment while preserving the canonical
    /// metric kernels and error behavior. Non-cosine metrics ignore
    /// `query_squared_norm`.
    ///
    /// # Errors
    ///
    /// [`VectorMetric::Cosine`] returns [`CoreError::VectorZeroNorm`] when the
    /// supplied query squared norm is not positive and finite.
    pub fn bind_query_with_squared_norm(
        self,
        query: &VectorValue,
        query_squared_norm: f64,
    ) -> CoreResult<VectorMetricQuery<'_>> {
        VectorMetricQuery::new_with_squared_norm(self, query, query_squared_norm)
    }

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

/// Metric scorer bound to a single query vector.
///
/// Use this when ranking many candidates against one query. It preserves the
/// same scores and error contract as [`VectorMetric::distance`], but avoids
/// recomputing query-only metric state for every candidate.
#[derive(Clone, Copy, Debug)]
pub struct VectorMetricQuery<'a> {
    metric: VectorMetric,
    query: &'a VectorValue,
    query_norm: Option<f64>,
}

impl<'a> VectorMetricQuery<'a> {
    fn new(metric: VectorMetric, query: &'a VectorValue) -> CoreResult<Self> {
        let query_norm = match metric {
            VectorMetric::SquaredEuclidean | VectorMetric::NegativeInnerProduct => None,
            VectorMetric::Cosine => {
                let norm = dot(query.as_slice(), query.as_slice());
                if norm == 0.0 {
                    return Err(CoreError::VectorZeroNorm { side: "lhs" });
                }
                Some(norm)
            }
        };
        Ok(Self {
            metric,
            query,
            query_norm,
        })
    }

    fn new_with_squared_norm(
        metric: VectorMetric,
        query: &'a VectorValue,
        query_squared_norm: f64,
    ) -> CoreResult<Self> {
        let query_norm = match metric {
            VectorMetric::SquaredEuclidean | VectorMetric::NegativeInnerProduct => None,
            VectorMetric::Cosine => Some(validate_precomputed_squared_norm(
                query_squared_norm,
                "lhs",
            )?),
        };
        Ok(Self {
            metric,
            query,
            query_norm,
        })
    }

    /// Return the metric this scorer uses.
    #[must_use]
    pub const fn metric(&self) -> VectorMetric {
        self.metric
    }

    /// Return the bound query vector.
    #[must_use]
    pub const fn query(&self) -> &'a VectorValue {
        self.query
    }

    /// Compute this bound query's lower-is-better distance to `candidate`.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::VectorDimensionMismatch`] if dimensions differ.
    /// [`VectorMetric::Cosine`] also returns [`CoreError::VectorZeroNorm`] when
    /// `candidate` has zero magnitude.
    pub fn distance(&self, candidate: &VectorValue) -> CoreResult<f64> {
        let query = self.query.as_slice();
        let candidate = candidate.as_slice();
        check_same_dimension(query.len(), candidate.len())?;
        Ok(canonical_score(match self.metric {
            VectorMetric::SquaredEuclidean => squared_euclidean(query, candidate),
            VectorMetric::Cosine => cosine_distance_with_lhs_norm(
                query,
                candidate,
                self.query_norm
                    .expect("cosine query scorer stores query norm"),
            )?,
            VectorMetric::NegativeInnerProduct => -dot(query, candidate),
        }))
    }

    /// Compute distance using a precomputed candidate squared norm.
    ///
    /// When `candidate_squared_norm` is the candidate's actual squared norm,
    /// this is equivalent to [`Self::distance`]. It lets ANN indexes cache
    /// centroid norms for cosine scoring while still using the canonical metric
    /// kernels and error behavior. Non-cosine metrics ignore
    /// `candidate_squared_norm`.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::VectorDimensionMismatch`] if dimensions differ.
    /// [`VectorMetric::Cosine`] returns [`CoreError::VectorZeroNorm`] when the
    /// supplied candidate squared norm is not positive and finite.
    pub fn distance_with_candidate_squared_norm(
        &self,
        candidate: &VectorValue,
        candidate_squared_norm: f64,
    ) -> CoreResult<f64> {
        let query = self.query.as_slice();
        let candidate = candidate.as_slice();
        check_same_dimension(query.len(), candidate.len())?;
        Ok(canonical_score(match self.metric {
            VectorMetric::SquaredEuclidean => squared_euclidean(query, candidate),
            VectorMetric::Cosine => cosine_distance_with_norms(
                query,
                candidate,
                self.query_norm
                    .expect("cosine query scorer stores query norm"),
                candidate_squared_norm,
            )?,
            VectorMetric::NegativeInnerProduct => -dot(query, candidate),
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

/// Bounded deterministic lower-is-better vector hit accumulator.
///
/// This is the streaming form of [`exact_vector_top_k`]. It keeps only the
/// current best `k` hits in memory, so graph-layer exact scans can avoid
/// materializing every candidate before ranking.
#[derive(Debug)]
pub struct VectorTopK<K> {
    k: usize,
    heap: BinaryHeap<HeapEntry<K>>,
}

impl<K: Ord> VectorTopK<K> {
    /// Construct an empty accumulator that will retain at most `k` hits.
    #[must_use]
    pub fn new(k: usize) -> Self {
        Self {
            k,
            heap: BinaryHeap::with_capacity(k),
        }
    }

    /// Return the configured result cap.
    #[must_use]
    pub const fn k(&self) -> usize {
        self.k
    }

    /// Return the number of retained hits.
    #[must_use]
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Return true when no hits are retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Push one candidate distance into the accumulator.
    ///
    /// `distance` must be a finite lower-is-better score produced by
    /// [`VectorMetric::distance`] or an equivalent metric kernel. Ties are
    /// deterministic: lower distance wins, then lower `key` wins.
    pub fn push_distance(&mut self, key: K, distance: f64) {
        debug_assert!(distance.is_finite(), "VectorTopK distances must be finite");
        if self.k == 0 {
            return;
        }
        let entry = HeapEntry { distance, key };
        if self.heap.len() < self.k {
            self.heap.push(entry);
            return;
        }
        let Some(worst) = self.heap.peek() else {
            return;
        };
        if entry.cmp(worst).is_lt() {
            self.heap.pop();
            self.heap.push(entry);
        }
    }

    /// Return retained hits sorted best-first.
    #[must_use]
    pub fn into_hits(self) -> Vec<VectorSearchHit<K>> {
        let mut hits: Vec<_> = self
            .heap
            .into_iter()
            .map(|entry| VectorSearchHit {
                key: entry.key,
                distance: entry.distance,
            })
            .collect();
        hits.sort_by(compare_hit);
        hits
    }
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

    let mut top_k = VectorTopK::new(k);
    let scorer = metric.bind_query(query)?;
    for (key, vector) in candidates {
        let distance = scorer.distance(vector)?;
        top_k.push_distance(key, distance);
    }

    Ok(top_k.into_hits())
}

/// Return `sum(component * component)` for a validated vector.
///
/// This is the shared squared-norm helper for ANN indexes that cache cosine
/// query or candidate norms. It intentionally uses the same chunked dot-product
/// kernel as the vector metric scorer.
#[must_use]
pub fn vector_squared_norm(vector: &VectorValue) -> f64 {
    dot(vector.as_slice(), vector.as_slice())
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
    fn bound_query_scores_match_one_off_distance() {
        let query = vector(&[1.0, 2.0, 3.0]);
        let candidate = vector(&[4.0, 5.0, 6.0]);

        for metric in [
            VectorMetric::SquaredEuclidean,
            VectorMetric::Cosine,
            VectorMetric::NegativeInnerProduct,
        ] {
            let scorer = metric.bind_query(&query).unwrap();
            assert_eq!(scorer.metric(), metric);
            assert_eq!(scorer.query(), &query);
            assert_eq!(
                scorer.distance(&candidate).unwrap(),
                metric.distance(&query, &candidate).unwrap()
            );
        }
    }

    #[test]
    fn bound_query_accepts_precomputed_candidate_norm() {
        let query = vector(&[1.0, 2.0, 3.0]);
        let candidate = vector(&[4.0, 5.0, 6.0]);
        let candidate_norm = dot(candidate.as_slice(), candidate.as_slice());

        let scorer = VectorMetric::Cosine.bind_query(&query).unwrap();

        assert_eq!(
            scorer
                .distance_with_candidate_squared_norm(&candidate, candidate_norm)
                .unwrap(),
            scorer.distance(&candidate).unwrap()
        );
    }

    #[test]
    fn bind_query_accepts_precomputed_query_norm() {
        let query = vector(&[1.0, 2.0, 3.0]);
        let candidate = vector(&[4.0, 5.0, 6.0]);
        let query_norm = dot(query.as_slice(), query.as_slice());

        let scorer = VectorMetric::Cosine
            .bind_query_with_squared_norm(&query, query_norm)
            .unwrap();

        assert_eq!(
            scorer.distance(&candidate).unwrap(),
            VectorMetric::Cosine
                .bind_query(&query)
                .unwrap()
                .distance(&candidate)
                .unwrap()
        );
    }

    #[test]
    fn vector_squared_norm_matches_component_sum() {
        let vector = vector(&[1.0, -2.0, 3.5]);

        assert_eq!(vector_squared_norm(&vector), 17.25);
    }

    #[test]
    fn bound_cosine_query_preserves_zero_norm_error_sides() {
        let zero = vector(&[0.0, 0.0]);
        let rhs = vector(&[1.0, 0.0]);

        let error = VectorMetric::Cosine.bind_query(&zero).unwrap_err();
        assert!(matches!(error, CoreError::VectorZeroNorm { side: "lhs" }));
        let error = VectorMetric::Cosine
            .bind_query_with_squared_norm(&rhs, 0.0)
            .unwrap_err();
        assert!(matches!(error, CoreError::VectorZeroNorm { side: "lhs" }));
        let error = VectorMetric::Cosine
            .bind_query_with_squared_norm(&rhs, f64::NAN)
            .unwrap_err();
        assert!(matches!(error, CoreError::VectorZeroNorm { side: "lhs" }));

        let scorer = VectorMetric::Cosine.bind_query(&rhs).unwrap();
        let error = scorer.distance(&zero).unwrap_err();
        assert!(matches!(error, CoreError::VectorZeroNorm { side: "rhs" }));

        let error = scorer
            .distance_with_candidate_squared_norm(&rhs, 0.0)
            .unwrap_err();
        assert!(matches!(error, CoreError::VectorZeroNorm { side: "rhs" }));
        let error = scorer
            .distance_with_candidate_squared_norm(&rhs, -1.0)
            .unwrap_err();
        assert!(matches!(error, CoreError::VectorZeroNorm { side: "rhs" }));
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

        let hits = exact_vector_top_k(VectorMetric::Cosine, &query, candidates, 0)
            .expect("zero k does not inspect candidates");

        assert!(hits.is_empty());
    }

    #[test]
    fn vector_top_k_streams_and_orders_hits() {
        let mut top_k = VectorTopK::new(2);
        top_k.push_distance(3_u64, 0.25);
        top_k.push_distance(1, 0.25);
        top_k.push_distance(2, 0.5);
        top_k.push_distance(4, 0.1);

        assert_eq!(top_k.k(), 2);
        assert_eq!(top_k.len(), 2);
        assert_eq!(
            top_k.into_hits(),
            vec![
                VectorSearchHit {
                    key: 4,
                    distance: 0.1
                },
                VectorSearchHit {
                    key: 1,
                    distance: 0.25
                }
            ]
        );
    }

    #[test]
    fn vector_top_k_zero_k_retains_nothing() {
        let mut top_k = VectorTopK::new(0);
        top_k.push_distance(1_u64, 0.0);

        assert!(top_k.is_empty());
        assert!(top_k.into_hits().is_empty());
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
