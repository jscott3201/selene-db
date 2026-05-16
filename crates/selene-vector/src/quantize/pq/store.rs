//! Dense PQ store and statistics.

use rkyv::{Archive, Deserialize, Serialize};

use crate::hnsw::distance::dot_product;
use crate::{DistanceMetric, PqParams, QuantMethod, VectorError, snapshot};

use super::{PQ_TRAIN_SEED, PqCodebook, validate_row};
use crate::quantize::{QuantizationStats, QuantizationStatsKind};

/// PQ quantized store in dense InternalIndex order.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct QuantizedStorePq {
    pub(crate) codebook: PqCodebook,
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
        let codebook = PqCodebook::train(dim, params, &rows, PQ_TRAIN_SEED, "qunt_pq_codebook")?;
        let mut codes = Vec::with_capacity(node_count * m);
        for row in &rows {
            codebook.encode_row(row, &mut codes);
        }
        let approx_norms = if metric == DistanceMetric::Cosine {
            Some(decoded_norms(&codebook, &codes))
        } else {
            None
        };

        Ok(Self {
            codebook,
            codes,
            approx_norms,
        })
    }

    pub(crate) fn node_count(&self) -> usize {
        let m = self.codebook.m_subspaces as usize;
        self.codes.len().checked_div(m).unwrap_or(0)
    }

    pub(crate) fn dim(&self) -> usize {
        self.codebook.dim()
    }

    pub(crate) fn bytes_codes(&self) -> usize {
        self.codes.len()
    }

    pub(crate) fn bytes_codebook(&self) -> usize {
        self.codebook.bytes_codebook()
    }

    pub(crate) fn bytes_rotation(&self) -> usize {
        self.codebook.bytes_rotation()
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
            .saturating_add(self.bytes_rotation())
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
                bytes_rotation: self.bytes_rotation(),
                polysemous: self.codebook.polysemous_trained,
            },
            bytes_norms: self.bytes_norms(),
            compression_ratio,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn build_query_lut(&self, query: &[f32], metric: DistanceMetric) -> Vec<f32> {
        self.codebook.build_query_lut(query, metric)
    }

    pub(crate) fn build_query_lut_into(
        &self,
        query: &[f32],
        metric: DistanceMetric,
        out: &mut Vec<f32>,
    ) {
        self.codebook.build_query_lut_into(query, metric, out);
    }

    pub(crate) fn lut_sum(&self, lut: &[f32], node_idx: usize) -> Option<f32> {
        let m = self.codebook.m_subspaces as usize;
        if node_idx >= self.node_count() {
            return None;
        }
        let row_start = node_idx.checked_mul(m)?;
        let row = self.codes.get(row_start..row_start.checked_add(m)?)?;
        self.codebook.lut_sum_for_codes(lut, row)
    }

    pub(crate) fn approx_norm(&self, node_idx: usize) -> Option<f32> {
        self.approx_norms
            .as_ref()
            .and_then(|norms| norms.get(node_idx).copied())
    }

    /// Return the PQ code bytes for `node_idx`, or `None` when the index is
    /// out of range. Used by the polysemous Hamming pre-filter to avoid
    /// rebuilding the LUT-sum slicing logic for what is fundamentally a
    /// byte-level read.
    pub(crate) fn codes_for(&self, node_idx: usize) -> Option<&[u8]> {
        let m = self.codebook.m_subspaces as usize;
        if node_idx >= self.node_count() {
            return None;
        }
        let row_start = node_idx.checked_mul(m)?;
        self.codes.get(row_start..row_start.checked_add(m)?)
    }

    /// Encode `query` against the (possibly polysemous-permuted) codebook,
    /// returning the byte representation used for Hamming pre-filtering.
    pub(crate) fn encode_query_codes(&self, query: &[f32]) -> Box<[u8]> {
        let m = self.codebook.m_subspaces as usize;
        let mut codes = Vec::with_capacity(m);
        self.codebook.encode_row(query, &mut codes);
        codes.into_boxed_slice()
    }

    /// Whether this store's codebook has been polysemous-permuted. The
    /// Hamming pre-filter remains a no-op when this returns `false`, so
    /// non-polysemous goldens cannot accidentally exercise the filter.
    pub(crate) fn polysemous_trained(&self) -> bool {
        self.codebook.polysemous_trained
    }

    #[cfg(test)]
    pub(crate) fn decode_row(&self, node_idx: usize, out: &mut [f32]) {
        let m = self.codebook.m_subspaces as usize;
        let row = &self.codes[node_idx * m..(node_idx + 1) * m];
        self.codebook
            .decode_codes(row, out)
            .expect("test row length matches codebook");
    }
}

fn decoded_norms(codebook: &PqCodebook, codes: &[u8]) -> Vec<f32> {
    let dim = codebook.dim();
    let mut decoded = vec![0.0; dim];
    let mut norms = Vec::with_capacity(
        codes
            .len()
            .checked_div(codebook.m_subspaces as usize)
            .unwrap_or(0),
    );
    for row in codes.chunks_exact(codebook.m_subspaces as usize) {
        codebook
            .decode_codes(row, &mut decoded)
            .expect("codes chunk length matches PQ subspaces");
        norms.push(dot_product(&decoded, &decoded).sqrt());
    }
    norms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_count_rotation_bytes_when_rotation_is_present() {
        let store = QuantizedStorePq {
            codebook: PqCodebook {
                m_subspaces: 2,
                k_centroids: 256,
                subspace_dim: 2,
                centroids: vec![0.0; 2 * 256 * 2],
                rotation: Some(crate::quantize::linalg::identity(4)),
                polysemous_trained: false,
            },
            codes: vec![0, 1, 2, 3],
            approx_norms: None,
        };

        let stats = store.stats();

        assert!(matches!(
            stats.kind,
            QuantizationStatsKind::Pq {
                bytes_codebook,
                bytes_rotation,
                polysemous: false,
            } if bytes_codebook == 2 * 256 * 2 * std::mem::size_of::<f32>()
                && bytes_rotation == 4 * 4 * std::mem::size_of::<f32>()
        ));
    }
}
