//! Product quantization (PQ) training, encoding, and ADC scoring.

use rkyv::{Archive, Deserialize, Serialize};

mod encode;
mod store;

pub(crate) use store::QuantizedStorePq;

use crate::clustering::kmeans_train_subspace;
use crate::quantize::{linalg, opq, polysemous};
use crate::{DistanceMetric, PqParams, VectorError, snapshot};

pub(crate) const PQ_TRAIN_SEED: u64 = 0xB66E_0001_u64;

/// Product-quantization codebook without dense row codes.
///
/// `polysemous_trained` records whether σ has been applied by
/// [`crate::quantize::polysemous`]. The flag is informational at runtime —
/// search-time Hamming filtering is gated by config, not by this flag — but
/// snapshot recovery rejects config drift when the decoded value disagrees
/// with `PqParams.use_polysemous` (V106 + V107).
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PqCodebook {
    pub(crate) m_subspaces: u32,
    pub(crate) k_centroids: u32,
    pub(crate) subspace_dim: u32,
    pub(crate) centroids: Vec<f32>,
    pub(crate) rotation: Option<Vec<f32>>,
    pub(crate) polysemous_trained: bool,
}

/// Decode-only pre-OPQ PQ codebook archive shape.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PqCodebookV1Legacy {
    pub(crate) m_subspaces: u32,
    pub(crate) k_centroids: u32,
    pub(crate) subspace_dim: u32,
    pub(crate) centroids: Vec<f32>,
}

/// Decode-only post-OPQ pre-polysemous PQ codebook archive shape. Used as the
/// encode shape whenever `polysemous_trained` is `false`, so snapshot goldens
/// stay byte-identical. V2 is the v1 shape extended with the OPQ rotation
/// field.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PqCodebookV2Legacy {
    pub(crate) m_subspaces: u32,
    pub(crate) k_centroids: u32,
    pub(crate) subspace_dim: u32,
    pub(crate) centroids: Vec<f32>,
    pub(crate) rotation: Option<Vec<f32>>,
}

impl From<PqCodebookV1Legacy> for PqCodebook {
    fn from(value: PqCodebookV1Legacy) -> Self {
        Self {
            m_subspaces: value.m_subspaces,
            k_centroids: value.k_centroids,
            subspace_dim: value.subspace_dim,
            centroids: value.centroids,
            rotation: None,
            polysemous_trained: false,
        }
    }
}

impl From<PqCodebookV2Legacy> for PqCodebook {
    fn from(value: PqCodebookV2Legacy) -> Self {
        Self {
            m_subspaces: value.m_subspaces,
            k_centroids: value.k_centroids,
            subspace_dim: value.subspace_dim,
            centroids: value.centroids,
            rotation: value.rotation,
            polysemous_trained: false,
        }
    }
}

impl PqCodebook {
    pub(crate) fn train(
        dim: usize,
        params: PqParams,
        rows: &[&[f32]],
        seed: u64,
        context: &'static str,
    ) -> Result<Self, VectorError> {
        params.validate_for_dim(dim)?;
        for row in rows {
            validate_row(row, dim)?;
        }
        if rows.len() < params.train_min_vectors {
            return Err(VectorError::PqTrainingDeferred {
                observed_vectors: rows.len(),
                required: params.train_min_vectors,
            });
        }
        // OPQ or plain k-means produces the codebook first. Polysemous runs
        // strictly LAST (V109) and only when `m_subspaces >= 2` (V108) — the
        // single-subspace case has no inter-subspace structure to optimize.
        let mut codebook = if params.use_opq && params.m_subspaces > 1 {
            opq::train(dim, params, rows, seed, context)?
        } else {
            Self::train_plain(dim, params, rows, seed, context, None)?
        };
        if params.use_polysemous && params.m_subspaces >= 2 {
            apply_polysemous(&mut codebook, context)?;
        }
        Ok(codebook)
    }

    pub(crate) fn train_plain(
        dim: usize,
        params: PqParams,
        rows: &[&[f32]],
        seed: u64,
        context: &'static str,
        rotation: Option<Vec<f32>>,
    ) -> Result<Self, VectorError> {
        params.validate_for_dim(dim)?;
        for row in rows {
            validate_row(row, dim)?;
        }
        if rows.len() < params.train_min_vectors {
            return Err(VectorError::PqTrainingDeferred {
                observed_vectors: rows.len(),
                required: params.train_min_vectors,
            });
        }
        if let Some(rotation) = rotation.as_deref() {
            validate_rotation(rotation, dim, context)?;
        }
        Self::train_plain_inner(dim, params, rows, seed, context, rotation)
    }

    #[cfg(test)]
    pub(crate) fn train_plain_relaxed_for_tests(
        dim: usize,
        params: PqParams,
        rows: &[&[f32]],
        seed: u64,
        context: &'static str,
        rotation: Option<Vec<f32>>,
    ) -> Result<Self, VectorError> {
        if params.m_subspaces == 0 || !dim.is_multiple_of(params.m_subspaces) {
            return Err(VectorError::PqDimensionNotDivisible {
                dim,
                m_subspaces: params.m_subspaces,
            });
        }
        if params.k_centroids == 0 || params.k_centroids > PqParams::K_CENTROIDS_V1 {
            return Err(VectorError::InvalidConfig {
                reason: format!(
                    "test PQ k_centroids must be in 1..={}, observed {}",
                    PqParams::K_CENTROIDS_V1,
                    params.k_centroids
                ),
            });
        }
        if rows.len() < params.train_min_vectors || rows.len() < params.k_centroids as usize {
            return Err(VectorError::PqTrainingDeferred {
                observed_vectors: rows.len(),
                required: params.train_min_vectors.max(params.k_centroids as usize),
            });
        }
        for row in rows {
            validate_row(row, dim)?;
        }
        if let Some(rotation) = rotation.as_deref() {
            validate_rotation(rotation, dim, context)?;
        }
        Self::train_plain_inner(dim, params, rows, seed, context, rotation)
    }

    fn train_plain_inner(
        dim: usize,
        params: PqParams,
        rows: &[&[f32]],
        seed: u64,
        context: &'static str,
        rotation: Option<Vec<f32>>,
    ) -> Result<Self, VectorError> {
        let m = params.m_subspaces;
        let k = params.k_centroids as usize;
        let subspace_dim = dim / m;
        let mut rng = fastrand::Rng::with_seed(seed);
        let mut centroids = Vec::with_capacity(m * k * subspace_dim);
        for subspace in 0..m {
            let start = subspace * subspace_dim;
            centroids.extend(kmeans_train_subspace(
                rows,
                start,
                subspace_dim,
                k,
                &mut rng,
            ));
        }

        Ok(Self {
            m_subspaces: u32::try_from(m).map_err(|_| VectorError::PqCodebookTrainFailed {
                context,
                reason: "PQ m_subspaces overflow".into(),
            })?,
            k_centroids: params.k_centroids,
            subspace_dim: u32::try_from(subspace_dim).map_err(|_| {
                VectorError::PqCodebookTrainFailed {
                    context,
                    reason: "PQ subspace_dim overflow".into(),
                }
            })?,
            centroids,
            rotation,
            polysemous_trained: false,
        })
    }

    #[cfg(test)]
    pub(crate) fn from_parts(
        m_subspaces: u32,
        k_centroids: u32,
        subspace_dim: u32,
        centroids: Vec<f32>,
    ) -> Self {
        Self {
            m_subspaces,
            k_centroids,
            subspace_dim,
            centroids,
            rotation: None,
            polysemous_trained: false,
        }
    }

    /// Render the codebook in the post-OPQ, pre-polysemous wire shape. Returns
    /// `None` when `polysemous_trained=true`; callers must emit the new
    /// flag-bearing archive in that case.
    pub(crate) fn as_v2_legacy(&self) -> Option<PqCodebookV2Legacy> {
        if self.polysemous_trained {
            return None;
        }
        Some(PqCodebookV2Legacy {
            m_subspaces: self.m_subspaces,
            k_centroids: self.k_centroids,
            subspace_dim: self.subspace_dim,
            centroids: self.centroids.clone(),
            rotation: self.rotation.clone(),
        })
    }

    pub(crate) fn dim(&self) -> usize {
        (self.m_subspaces as usize).saturating_mul(self.subspace_dim as usize)
    }

    pub(crate) fn bytes_codebook(&self) -> usize {
        self.centroids.len() * std::mem::size_of::<f32>()
    }

    pub(crate) fn bytes_rotation(&self) -> usize {
        self.rotation
            .as_ref()
            .map_or(0, |rotation| rotation.len() * std::mem::size_of::<f32>())
    }

    pub(crate) fn encode_row(&self, row: &[f32], out: &mut Vec<u8>) {
        let m = self.m_subspaces as usize;
        let k = self.k_centroids as usize;
        let subdim = self.subspace_dim as usize;
        if let Some(rotation) = self.rotation.as_deref() {
            let rotated = linalg::mat_vec(rotation, row, self.dim());
            encode::encode_row(&rotated, &self.centroids, m, k, subdim, out);
        } else {
            encode::encode_row(row, &self.centroids, m, k, subdim, out);
        }
    }

    pub(crate) fn build_query_lut(&self, query: &[f32], metric: DistanceMetric) -> Vec<f32> {
        let mut lut = Vec::new();
        self.build_query_lut_into(query, metric, &mut lut);
        lut
    }

    pub(crate) fn build_query_lut_into(
        &self,
        query: &[f32],
        metric: DistanceMetric,
        out: &mut Vec<f32>,
    ) {
        debug_assert_eq!(query.len(), self.dim(), "PQ query LUT dimension mismatch");
        let m = self.m_subspaces as usize;
        let k = self.k_centroids as usize;
        let subdim = self.subspace_dim as usize;
        if let Some(rotation) = self.rotation.as_deref() {
            let rotated = linalg::mat_vec(rotation, query, self.dim());
            encode::build_query_lut_into(&rotated, &self.centroids, m, k, subdim, metric, out);
        } else {
            encode::build_query_lut_into(query, &self.centroids, m, k, subdim, metric, out);
        }
    }

    pub(crate) fn lut_sum_for_codes(&self, lut: &[f32], codes: &[u8]) -> Option<f32> {
        let m = self.m_subspaces as usize;
        let k = self.k_centroids as usize;
        encode::lut_sum_for_codes(lut, codes, m, k)
    }

    pub(crate) fn decode_codes(&self, codes: &[u8], out: &mut [f32]) -> Option<()> {
        let m = self.m_subspaces as usize;
        let k = self.k_centroids as usize;
        let subdim = self.subspace_dim as usize;
        if self.rotation.is_none() {
            return encode::decode_codes(&self.centroids, codes, m, k, subdim, out);
        }
        let mut rotated = vec![0.0; self.dim()];
        encode::decode_codes(&self.centroids, codes, m, k, subdim, &mut rotated)?;
        let decoded = linalg::mat_transpose_vec(self.rotation.as_deref()?, &rotated, self.dim());
        out.copy_from_slice(&decoded);
        Some(())
    }
}

pub(crate) fn validate_rotation(
    rotation: &[f32],
    dim: usize,
    context: &'static str,
) -> Result<(), VectorError> {
    if rotation.len() != dim.saturating_mul(dim) {
        return Err(VectorError::OpqTrainingFailed {
            context,
            reason: format!(
                "OPQ rotation length {} disagrees with dim^2 {}",
                rotation.len(),
                dim.saturating_mul(dim)
            ),
        });
    }
    for (index, value) in rotation.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(VectorError::OpqTrainingFailed {
                context,
                reason: format!("non-finite OPQ rotation component at index {index}: {value}"),
            });
        }
    }
    if !linalg::is_orthonormal(rotation, dim, 1.0e-5) {
        return Err(VectorError::OpqTrainingFailed {
            context,
            reason: "OPQ rotation is not orthonormal".into(),
        });
    }
    Ok(())
}

/// Apply polysemous σ to a trained codebook in place, setting the
/// `polysemous_trained` flag. Caller is responsible for verifying
/// `params.use_polysemous && m_subspaces >= 2` before invoking.
fn apply_polysemous(codebook: &mut PqCodebook, context: &'static str) -> Result<(), VectorError> {
    let m = codebook.m_subspaces as usize;
    let k = codebook.k_centroids as usize;
    let subspace_dim = codebook.subspace_dim as usize;
    let sigmas = polysemous::train(
        &codebook.centroids,
        m,
        k,
        subspace_dim,
        polysemous::POLYSEMOUS_TRAIN_SEED,
        context,
    )?;
    polysemous::apply_permutation(
        &mut codebook.centroids,
        &sigmas,
        m,
        k,
        subspace_dim,
        context,
    )?;
    codebook.polysemous_trained = true;
    Ok(())
}

pub(super) fn validate_row(row: &[f32], dim: usize) -> Result<(), VectorError> {
    if row.len() != dim {
        return Err(VectorError::DimensionMismatch {
            expected: dim,
            observed: row.len(),
        });
    }
    for (index, value) in row.iter().copied().enumerate() {
        debug_assert!(
            value.is_finite(),
            "wire validators are authoritative for finite vector components"
        );
        if !value.is_finite() {
            return Err(snapshot::encode_failed(
                snapshot::QUNT,
                format!("non-finite vector component at coordinate {index}: {value}"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clustering::squared_l2;
    use crate::hnsw::distance::dot_product;

    fn params(dim: usize) -> PqParams {
        PqParams {
            m_subspaces: (dim / 2).max(1),
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq: false,
            use_polysemous: false,
            hamming_threshold_ratio: 0.5,
        }
    }

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
    fn pq_train_seed_pinned() {
        assert_eq!(PQ_TRAIN_SEED, 0xB66E_0001_u64);
    }

    #[test]
    fn pq_default_m_subspaces_derives_from_dim() {
        assert_eq!(PqParams::default_for_dim(128).m_subspaces, 16);
        assert_eq!(PqParams::default_for_dim(1).m_subspaces, 1);
    }

    #[test]
    fn pq_encode_decode_with_rotation_distinguishes_direction() {
        let angle = std::f32::consts::FRAC_PI_6;
        let (s, c) = angle.sin_cos();
        let codebook = PqCodebook {
            m_subspaces: 2,
            k_centroids: 2,
            subspace_dim: 1,
            centroids: vec![0.0, c, -s, s],
            rotation: Some(vec![c, -s, s, c]),
            polysemous_trained: false,
        };
        let mut codes = Vec::new();

        codebook.encode_row(&[1.0, 0.0], &mut codes);

        assert_eq!(codes, vec![1, 1]);
        let mut decoded = vec![0.0; 2];
        codebook.decode_codes(&codes, &mut decoded).unwrap();
        assert!((decoded[0] - 1.0).abs() <= 1.0e-6);
        assert!(decoded[1].abs() <= 1.0e-6);
    }

    #[test]
    fn cosine_lut_asymmetric_score_matches_hand_derived() {
        let codebook = PqCodebook {
            m_subspaces: 1,
            k_centroids: 1,
            subspace_dim: 2,
            centroids: vec![0.0, 5.0],
            rotation: None,
            polysemous_trained: false,
        };
        let query = [3.0, 0.0];
        let lut = codebook.build_query_lut(&query, DistanceMetric::Cosine);
        let lut_sum = codebook.lut_sum_for_codes(&lut, &[0]).unwrap();
        let query_norm = dot_product(&query, &query).sqrt();
        let approx_norm = 5.0_f32;

        let score = (lut_sum / (query_norm * approx_norm)) - 1.0;

        assert_eq!(lut_sum, 0.0);
        assert!((score + 1.0).abs() <= f32::EPSILON);
    }

    #[test]
    fn pq_build_is_deterministic_given_seed() {
        let rows = rows(256, 8);
        let refs = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();

        let left =
            QuantizedStorePq::build(256, 8, DistanceMetric::L2, params(8), refs.iter().copied())
                .unwrap();
        let right =
            QuantizedStorePq::build(256, 8, DistanceMetric::L2, params(8), refs.iter().copied())
                .unwrap();

        assert_eq!(left.codebook, right.codebook);
        assert_eq!(left.codes, right.codes);
    }

    #[test]
    fn pq_encode_decode_round_trip_uses_centroid_values() {
        let rows = rows(256, 4);
        let refs = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let store =
            QuantizedStorePq::build(256, 4, DistanceMetric::L2, params(4), refs.iter().copied())
                .unwrap();
        let mut decoded = vec![0.0; 4];

        store.decode_row(0, &mut decoded);

        assert!(decoded.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn pq_encode_rejects_dim_mismatch() {
        let rows = rows(256, 4);
        let refs = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();

        let err =
            QuantizedStorePq::build(256, 5, DistanceMetric::L2, params(5), refs.iter().copied())
                .expect_err("dim mismatch rejected");

        assert!(matches!(
            err,
            VectorError::PqDimensionNotDivisible { .. } | VectorError::DimensionMismatch { .. }
        ));
    }

    #[test]
    fn adc_lut_l2_matches_decoded_distance() {
        let rows = rows(256, 4);
        let refs = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let store =
            QuantizedStorePq::build(256, 4, DistanceMetric::L2, params(4), refs.iter().copied())
                .unwrap();
        let query = [0.2, -0.4, 0.1, 0.8];
        let lut = store.build_query_lut(&query, DistanceMetric::L2);
        let lut_sum = store.lut_sum(&lut, 0).unwrap();
        let mut decoded = vec![0.0; 4];
        store.decode_row(0, &mut decoded);
        let direct = squared_l2(&query, &decoded);

        assert!((lut_sum - direct).abs() <= 1.0e-5);
    }

    #[test]
    fn adc_lut_dot_matches_decoded_dot() {
        let rows = rows(256, 4);
        let refs = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let store =
            QuantizedStorePq::build(256, 4, DistanceMetric::Dot, params(4), refs.iter().copied())
                .unwrap();
        let query = [0.2, -0.4, 0.1, 0.8];
        let lut = store.build_query_lut(&query, DistanceMetric::Dot);
        let lut_sum = store.lut_sum(&lut, 0).unwrap();
        let mut decoded = vec![0.0; 4];
        store.decode_row(0, &mut decoded);
        let direct = dot_product(&query, &decoded);

        assert!((lut_sum - direct).abs() <= 1.0e-5);
    }

    #[test]
    fn pq_cosine_builds_approx_norm_cache() {
        let rows = rows(256, 4);
        let refs = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let store = QuantizedStorePq::build(
            256,
            4,
            DistanceMetric::Cosine,
            params(4),
            refs.iter().copied(),
        )
        .unwrap();

        assert_eq!(store.approx_norms.as_ref().unwrap().len(), 256);
    }

    #[test]
    fn opq_training_returns_valid_multiple_subspace_codebook() {
        let rows = rows(256, 8);
        let refs = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let params = PqParams {
            m_subspaces: 2,
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq: true,
            use_polysemous: false,
            hamming_threshold_ratio: 0.5,
        };

        let codebook = PqCodebook::train(8, params, &refs, PQ_TRAIN_SEED, "test_opq").unwrap();

        if let Some(rotation) = codebook.rotation.as_deref() {
            assert!(crate::quantize::linalg::is_orthonormal(rotation, 8, 1.0e-5));
        }
        assert_eq!(codebook.dim(), 8);
    }

    #[test]
    fn opq_with_one_subspace_short_circuits_to_none_rotation() {
        let rows = rows(256, 8);
        let refs = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let params = PqParams {
            m_subspaces: 1,
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq: true,
            use_polysemous: false,
            hamming_threshold_ratio: 0.5,
        };

        let codebook = PqCodebook::train(8, params, &refs, PQ_TRAIN_SEED, "test_opq").unwrap();

        assert!(codebook.rotation.is_none());
    }

    #[test]
    fn kmeans_empty_cluster_policy_is_deterministic() {
        let rows = [
            vec![0.0, 0.0],
            vec![0.0, 0.0],
            vec![4.0, 4.0],
            vec![8.0, 8.0],
        ];
        let refs = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let mut left_rng = fastrand::Rng::with_seed(PQ_TRAIN_SEED);
        let mut right_rng = fastrand::Rng::with_seed(PQ_TRAIN_SEED);

        let left = kmeans_train_subspace(&refs, 0, 2, 4, &mut left_rng);
        let right = kmeans_train_subspace(&refs, 0, 2, 4, &mut right_rng);

        assert_eq!(left, right);
    }
}
