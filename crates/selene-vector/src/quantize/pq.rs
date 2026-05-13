//! Product quantization (PQ) training, encoding, and ADC scoring.

use rkyv::{Archive, Deserialize, Serialize};

use crate::clustering::{kmeans_train_subspace, nearest_centroid, squared_l2};
use crate::hnsw::distance::dot_product;
use crate::{DistanceMetric, PqParams, QuantMethod, VectorError, snapshot};

use super::{QuantizationStats, QuantizationStatsKind};

pub(crate) const PQ_TRAIN_SEED: u64 = 0xB66E_0001_u64;

/// Product-quantization codebook without dense row codes.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PqCodebook {
    pub(crate) m_subspaces: u32,
    pub(crate) k_centroids: u32,
    pub(crate) subspace_dim: u32,
    pub(crate) centroids: Vec<f32>,
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
        })
    }

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
        }
    }

    pub(crate) fn dim(&self) -> usize {
        (self.m_subspaces as usize).saturating_mul(self.subspace_dim as usize)
    }

    pub(crate) fn bytes_codebook(&self) -> usize {
        self.centroids.len() * std::mem::size_of::<f32>()
    }

    pub(crate) fn encode_row(&self, row: &[f32], out: &mut Vec<u8>) {
        let m = self.m_subspaces as usize;
        let k = self.k_centroids as usize;
        let subdim = self.subspace_dim as usize;
        encode_row(row, &self.centroids, m, k, subdim, out);
    }

    pub(crate) fn build_query_lut(&self, query: &[f32], metric: DistanceMetric) -> Vec<f32> {
        debug_assert_eq!(query.len(), self.dim(), "PQ query LUT dimension mismatch");
        let m = self.m_subspaces as usize;
        let k = self.k_centroids as usize;
        let subdim = self.subspace_dim as usize;
        let mut lut = vec![0.0; m * k];
        for subspace in 0..m {
            let query_start = subspace * subdim;
            let query_slice = &query[query_start..query_start + subdim];
            for centroid in 0..k {
                let center = self.centroid(subspace, centroid);
                lut[(subspace * k) + centroid] = match metric {
                    DistanceMetric::Cosine | DistanceMetric::Dot => {
                        dot_product(query_slice, center)
                    }
                    DistanceMetric::L2 => squared_l2(query_slice, center),
                };
            }
        }
        lut
    }

    pub(crate) fn lut_sum_for_codes(&self, lut: &[f32], codes: &[u8]) -> Option<f32> {
        let m = self.m_subspaces as usize;
        let k = self.k_centroids as usize;
        if codes.len() != m || lut.len() != m.checked_mul(k)? {
            return None;
        }
        Some(
            codes
                .iter()
                .copied()
                .enumerate()
                .map(|(subspace, code)| lut[(subspace * k) + usize::from(code)])
                .sum(),
        )
    }

    pub(crate) fn decode_codes(&self, codes: &[u8], out: &mut [f32]) -> Option<()> {
        let m = self.m_subspaces as usize;
        let subdim = self.subspace_dim as usize;
        if codes.len() != m || out.len() != self.dim() {
            return None;
        }
        for (subspace, code) in codes.iter().copied().enumerate() {
            let start = subspace * subdim;
            out[start..start + subdim].copy_from_slice(self.centroid(subspace, usize::from(code)));
        }
        Some(())
    }

    fn centroid(&self, subspace: usize, centroid: usize) -> &[f32] {
        let k = self.k_centroids as usize;
        let subdim = self.subspace_dim as usize;
        let start = ((subspace * k) + centroid) * subdim;
        &self.centroids[start..start + subdim]
    }
}

/// PQ quantized store in dense InternalIndex order.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct QuantizedStorePq {
    pub(crate) m_subspaces: u32,
    pub(crate) k_centroids: u32,
    pub(crate) subspace_dim: u32,
    pub(crate) codebook: Vec<f32>,
    pub(crate) codes: Vec<u8>,
    pub(crate) approx_norms: Option<Vec<f32>>,
}

impl QuantizedStorePq {
    pub(crate) fn build<'a>(
        node_count: usize,
        dim: usize,
        metric: DistanceMetric,
        params: PqParams,
        vectors: impl Iterator<Item = &'a [f32]>,
    ) -> Result<Self, VectorError> {
        params.validate_for_dim(dim)?;
        let rows = vectors.collect::<Vec<_>>();
        if rows.len() != node_count {
            return Err(snapshot::encode_failed(
                snapshot::QUNT,
                format!("expected {node_count} vectors, observed {}", rows.len()),
            ));
        }
        if rows.len() < params.train_min_vectors {
            return Err(VectorError::PqTrainingDeferred {
                observed_vectors: rows.len(),
                required: params.train_min_vectors,
            });
        }
        for row in &rows {
            validate_row(row, dim)?;
        }

        let m = params.m_subspaces;
        let k = params.k_centroids as usize;
        let subspace_dim = dim / m;
        let codebook = PqCodebook::train(dim, params, &rows, PQ_TRAIN_SEED, "qunt_pq_codebook")?;
        let mut codes = Vec::with_capacity(node_count * m);
        for row in &rows {
            codebook.encode_row(row, &mut codes);
        }
        let approx_norms = if metric == DistanceMetric::Cosine {
            Some(decoded_norms(
                &codebook.centroids,
                &codes,
                m,
                k,
                subspace_dim,
            ))
        } else {
            None
        };

        Ok(Self {
            m_subspaces: u32::try_from(m)
                .map_err(|_| snapshot::encode_failed(snapshot::QUNT, "PQ m_subspaces overflow"))?,
            k_centroids: params.k_centroids,
            subspace_dim: u32::try_from(subspace_dim)
                .map_err(|_| snapshot::encode_failed(snapshot::QUNT, "PQ subspace_dim overflow"))?,
            codebook: codebook.centroids,
            codes,
            approx_norms,
        })
    }

    pub(crate) fn node_count(&self) -> usize {
        let m = self.m_subspaces as usize;
        self.codes.len().checked_div(m).unwrap_or(0)
    }

    pub(crate) fn dim(&self) -> usize {
        (self.m_subspaces as usize).saturating_mul(self.subspace_dim as usize)
    }

    pub(crate) fn bytes_codes(&self) -> usize {
        self.codes.len()
    }

    pub(crate) fn bytes_codebook(&self) -> usize {
        self.codebook.len() * std::mem::size_of::<f32>()
    }

    pub(crate) fn bytes_norms(&self) -> usize {
        self.approx_norms
            .as_ref()
            .map_or(0, |norms| norms.len() * std::mem::size_of::<f32>())
    }

    pub(crate) fn stats(&self) -> QuantizationStats {
        let f32_bytes = self
            .node_count()
            .saturating_mul(self.dim())
            .saturating_mul(std::mem::size_of::<f32>());
        let quantized_bytes = self
            .bytes_codes()
            .saturating_add(self.bytes_codebook())
            .saturating_add(self.bytes_norms());
        let compression_ratio = if quantized_bytes == 0 {
            0.0
        } else {
            f32_bytes as f32 / quantized_bytes as f32
        };

        QuantizationStats {
            method: QuantMethod::Pq,
            dim: self.dim(),
            code_count: self.node_count(),
            bytes_codes: self.bytes_codes(),
            kind: QuantizationStatsKind::Pq {
                bytes_codebook: self.bytes_codebook(),
            },
            bytes_norms: self.bytes_norms(),
            compression_ratio,
        }
    }

    pub(crate) fn build_query_lut(&self, query: &[f32], metric: DistanceMetric) -> Vec<f32> {
        debug_assert_eq!(query.len(), self.dim(), "PQ query LUT dimension mismatch");
        PqCodebook::from_parts(
            self.m_subspaces,
            self.k_centroids,
            self.subspace_dim,
            self.codebook.clone(),
        )
        .build_query_lut(query, metric)
    }

    pub(crate) fn lut_sum(&self, lut: &[f32], node_idx: usize) -> Option<f32> {
        let m = self.m_subspaces as usize;
        let k = self.k_centroids as usize;
        if node_idx >= self.node_count() || lut.len() != m.checked_mul(k)? {
            return None;
        }
        let row_start = node_idx.checked_mul(m)?;
        let row = self.codes.get(row_start..row_start.checked_add(m)?)?;
        Some(
            row.iter()
                .copied()
                .enumerate()
                .map(|(subspace, code)| lut[(subspace * k) + usize::from(code)])
                .sum(),
        )
    }

    pub(crate) fn approx_norm(&self, node_idx: usize) -> Option<f32> {
        self.approx_norms
            .as_ref()
            .and_then(|norms| norms.get(node_idx).copied())
    }

    #[cfg(test)]
    fn centroid(&self, subspace: usize, centroid: usize) -> &[f32] {
        let k = self.k_centroids as usize;
        let subdim = self.subspace_dim as usize;
        let start = ((subspace * k) + centroid) * subdim;
        &self.codebook[start..start + subdim]
    }

    #[cfg(test)]
    fn decode_row(&self, node_idx: usize, out: &mut [f32]) {
        let m = self.m_subspaces as usize;
        let subdim = self.subspace_dim as usize;
        let row = &self.codes[node_idx * m..(node_idx + 1) * m];
        for (subspace, code) in row.iter().copied().enumerate() {
            let start = subspace * subdim;
            out[start..start + subdim].copy_from_slice(self.centroid(subspace, usize::from(code)));
        }
    }
}

fn validate_row(row: &[f32], dim: usize) -> Result<(), VectorError> {
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

fn encode_row(row: &[f32], codebook: &[f32], m: usize, k: usize, subdim: usize, out: &mut Vec<u8>) {
    for subspace in 0..m {
        let start = subspace * subdim;
        let codebook_start = subspace * k * subdim;
        let code = nearest_centroid(
            &row[start..start + subdim],
            &codebook[codebook_start..codebook_start + (k * subdim)],
            k,
            subdim,
        );
        out.push(u8::try_from(code).expect("BRIEF-66 fixes PQ K to 256"));
    }
}

fn decoded_norms(codebook: &[f32], codes: &[u8], m: usize, k: usize, subdim: usize) -> Vec<f32> {
    codes
        .chunks_exact(m)
        .map(|row| {
            let mut sum = 0.0;
            for (subspace, code) in row.iter().copied().enumerate() {
                let start = ((subspace * k) + usize::from(code)) * subdim;
                let centroid = &codebook[start..start + subdim];
                sum += dot_product(centroid, centroid);
            }
            sum.sqrt()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(dim: usize) -> PqParams {
        PqParams {
            m_subspaces: (dim / 2).max(1),
            k_centroids: 256,
            train_min_vectors: 256,
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
