//! Quantized vector stores used by the QUNT snapshot overlay.

use rkyv::{Archive, Deserialize, Serialize};

use crate::{DistanceMetric, VectorError, snapshot};

pub(crate) mod linalg;
mod opq;
mod polysemous;
mod pq;
mod sq8;

pub(crate) use pq::{PqCodebook, PqCodebookV1Legacy, PqCodebookV2Legacy, QuantizedStorePq};
#[cfg(test)]
pub(crate) use sq8::PerCoordRange;
pub(crate) use sq8::QuantizedStoreSq8;

/// Maximum vector dimension for OPQ rotation training.
pub const OPQ_MAX_DIM: usize = 1024;

/// Return the maximum vector dimension that permits OPQ training.
#[must_use]
pub const fn opq_max_dim() -> usize {
    OPQ_MAX_DIM
}

/// Quantization method tag for persisted quantized vector stores.
#[derive(Archive, Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[repr(u8)]
pub enum QuantMethod {
    /// Per-coordinate 8-bit scalar quantization.
    #[default]
    Sq8 = 0,
    /// Product quantization with 8-bit centroid codes per subspace.
    Pq = 1,
}

impl QuantMethod {
    pub(crate) const fn to_wire(self) -> u8 {
        match self {
            Self::Sq8 => 0,
            Self::Pq => 1,
        }
    }

    pub(crate) fn from_wire(value: u8) -> Result<Self, VectorError> {
        match value {
            0 => Ok(Self::Sq8),
            1 => Ok(Self::Pq),
            other => Err(snapshot::decode_failed(
                snapshot::QUNT,
                format!("unknown QUNT method {other}"),
            )),
        }
    }
}

/// Product-quantization training and encoding parameters.
// Eq is intentionally NOT derived: `hamming_threshold_ratio: f32` cascades
// non-Eq semantics through QuantizationConfig, HnswConfig, and IvfConfig
// per BRIEF-69 §C.6 F7. NaN is rejected by `validate_for_dim`, so
// `PartialEq` is the contract callers should use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PqParams {
    /// Number of equal-width vector subspaces.
    pub m_subspaces: usize,
    /// Number of centroids per subspace. BRIEF-66 fixes this to 256.
    pub k_centroids: u32,
    /// Minimum vector count required before PQ training may run.
    pub train_min_vectors: usize,
    /// Train an OPQ rotation alongside the PQ codebook when dimensions permit.
    pub use_opq: bool,
    /// Train polysemous-codes permutation when `m_subspaces >= 2`.
    /// `default_for_dim` enables this for any default config with
    /// `m_subspaces >= 2`; embedders can opt out by setting it to `false`.
    /// IVF v1 applies the search-time Hamming pre-filter only for `L2`
    /// because Dot/Cosine score raw-query terms against residual posting
    /// codes, which are not comparable Hamming frames.
    pub use_polysemous: bool,
    /// Hamming distance threshold as a fraction of the maximum (`8 * m`).
    /// Search rejects candidates whose query/stored Hamming exceeds
    /// `round(ratio * 8 * m_subspaces)`. Must be finite and in `[0.0, 1.0]`.
    pub hamming_threshold_ratio: f32,
}

impl PqParams {
    /// Fixed v1 centroid count. One code byte selects one centroid.
    pub const K_CENTROIDS_V1: u32 = 256;

    /// Default Hamming-threshold ratio when polysemous is enabled.
    ///
    /// Skip candidates whose Hamming distance from the query is more than
    /// half the maximum (`4 * m_subspaces` bits). FAISS uses this midpoint
    /// as a moderate default that prunes roughly half of candidates while
    /// keeping recall@10 within 1–2% of unfiltered.
    pub const DEFAULT_HAMMING_THRESHOLD_RATIO: f32 = 0.5;

    /// Derive the default PQ parameters for `dim`.
    #[must_use]
    pub fn default_for_dim(dim: usize) -> Self {
        let target = (dim / 8).max(1);
        let m_subspaces = (1..=target)
            .rev()
            .find(|m| dim.is_multiple_of(*m))
            .unwrap_or(1);
        Self {
            m_subspaces,
            k_centroids: Self::K_CENTROIDS_V1,
            train_min_vectors: Self::K_CENTROIDS_V1 as usize,
            use_opq: dim <= OPQ_MAX_DIM,
            // Polysemous needs ≥ 2 subspaces to optimize against inter-
            // subspace Hamming structure (V108). For single-subspace
            // codebooks the permutation is degenerate; default off.
            use_polysemous: m_subspaces >= 2,
            hamming_threshold_ratio: Self::DEFAULT_HAMMING_THRESHOLD_RATIO,
        }
    }

    /// Resolve an optional override against the BRIEF-66 defaults.
    #[must_use]
    pub fn resolve(dim: usize, override_params: Option<Self>) -> Self {
        override_params.unwrap_or_else(|| Self::default_for_dim(dim))
    }

    pub(crate) fn validate_for_dim(self, dim: usize) -> Result<(), VectorError> {
        if self.m_subspaces == 0 || self.m_subspaces > dim {
            return Err(VectorError::InvalidConfig {
                reason: format!(
                    "PQ m_subspaces must be in 1..={dim}, observed {}",
                    self.m_subspaces
                ),
            });
        }
        if !dim.is_multiple_of(self.m_subspaces) {
            return Err(VectorError::PqDimensionNotDivisible {
                dim,
                m_subspaces: self.m_subspaces,
            });
        }
        if self.k_centroids != Self::K_CENTROIDS_V1 {
            return Err(VectorError::InvalidConfig {
                reason: format!(
                    "PQ k_centroids must be {}, observed {}",
                    Self::K_CENTROIDS_V1,
                    self.k_centroids
                ),
            });
        }
        if self.train_min_vectors < self.k_centroids as usize {
            return Err(VectorError::InvalidConfig {
                reason: format!(
                    "PQ train_min_vectors must be at least {}, observed {}",
                    self.k_centroids, self.train_min_vectors
                ),
            });
        }
        if self.use_opq && dim > OPQ_MAX_DIM {
            return Err(VectorError::InvalidConfig {
                reason: format!("OPQ requires dim <= OPQ_MAX_DIM ({OPQ_MAX_DIM}), observed {dim}"),
            });
        }
        // V108: single-subspace codebooks have no inter-subspace Hamming
        // structure to optimize. An explicit `use_polysemous=true` with
        // `m_subspaces=1` is a config error, not a silent short-circuit.
        if self.use_polysemous && self.m_subspaces < 2 {
            return Err(VectorError::InvalidConfig {
                reason: format!(
                    "polysemous requires m_subspaces >= 2, observed {}",
                    self.m_subspaces
                ),
            });
        }
        // V112 + F7: the ratio must be finite and in [0.0, 1.0]. NaN and
        // out-of-range values are rejected (not clamped) so a bad config
        // surfaces immediately at validate time rather than as silent
        // recall regression at search time.
        if !self.hamming_threshold_ratio.is_finite() {
            return Err(VectorError::InvalidConfig {
                reason: format!(
                    "polysemous hamming_threshold_ratio must be finite, observed {}",
                    self.hamming_threshold_ratio
                ),
            });
        }
        if !(0.0..=1.0).contains(&self.hamming_threshold_ratio) {
            return Err(VectorError::InvalidConfig {
                reason: format!(
                    "polysemous hamming_threshold_ratio must be in [0.0, 1.0], observed {}",
                    self.hamming_threshold_ratio
                ),
            });
        }
        Ok(())
    }

    /// Resolve `hamming_threshold_ratio` to an integer Hamming distance over
    /// `m_subspaces * 8` bits. Used by both HNSW and IVF Hamming filters
    /// (V112).
    #[must_use]
    pub fn resolve_hamming_threshold(self) -> u32 {
        let max_bits = self.m_subspaces.saturating_mul(8) as f32;
        (self.hamming_threshold_ratio * max_bits).round() as u32
    }
}

/// User-facing quantization configuration for [`crate::HnswConfig`].
// Eq dropped because `PqParams` now carries an f32 `hamming_threshold_ratio`
// (BRIEF-69 §C.6 F7). Callers should compare via `PartialEq`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QuantizationConfig {
    /// Enable asymmetric quantized search when a QUNT store is loaded.
    pub enabled: bool,
    /// Quantization method selected when quantization is enabled.
    pub method: QuantMethod,
    /// Re-rank the asymmetric candidate set with exact f32 scoring.
    pub rescore: bool,
    /// Optional product-quantization parameters. Ignored unless
    /// [`Self::method`] is [`QuantMethod::Pq`].
    pub pq: Option<PqParams>,
}

/// Method-specific storage accounting for [`QuantizationStats`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum QuantizationStatsKind {
    /// SQ8 range table bytes.
    Sq8 {
        /// Bytes used by per-coordinate min/max ranges.
        bytes_ranges: usize,
    },
    /// PQ codebook bytes.
    Pq {
        /// Bytes used by learned PQ centroids.
        bytes_codebook: usize,
        /// Bytes used by an optional OPQ rotation matrix.
        bytes_rotation: usize,
        /// V116: whether the trained codebook is polysemous-permuted.
        polysemous: bool,
    },
}

/// Lightweight statistics for a loaded quantized store.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuantizationStats {
    /// Quantization method used by the loaded store.
    pub method: QuantMethod,
    /// Vector dimensionality.
    pub dim: usize,
    /// Number of quantized vectors.
    pub code_count: usize,
    /// Bytes used by packed quantized codes.
    pub bytes_codes: usize,
    /// Method-specific byte accounting.
    pub kind: QuantizationStatsKind,
    /// Bytes used by approximate norm cache.
    pub bytes_norms: usize,
    /// `f32_bytes / quantized_bytes`, including method-specific side data.
    pub compression_ratio: f32,
}

/// Quantized store variants in dense InternalIndex order.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) enum QuantizedStore {
    /// Per-coordinate scalar quantization.
    Sq8(QuantizedStoreSq8),
    /// Product quantization.
    Pq(QuantizedStorePq),
}

impl QuantizedStore {
    pub(crate) fn build<'a>(
        node_count: usize,
        dim: usize,
        metric: DistanceMetric,
        config: QuantizationConfig,
        vectors: impl Iterator<Item = &'a [f32]>,
    ) -> Result<Option<Self>, VectorError> {
        if !config.enabled {
            return Ok(None);
        }
        match config.method {
            QuantMethod::Sq8 => Ok(Some(Self::Sq8(QuantizedStoreSq8::build(
                node_count, dim, vectors,
            )?))),
            QuantMethod::Pq => {
                let params = PqParams::resolve(dim, config.pq);
                if node_count < params.train_min_vectors {
                    return Ok(None);
                }
                Ok(Some(Self::Pq(QuantizedStorePq::build(
                    node_count, dim, metric, params, vectors,
                )?)))
            }
        }
    }

    pub(crate) fn method(&self) -> QuantMethod {
        match self {
            Self::Sq8(_) => QuantMethod::Sq8,
            Self::Pq(_) => QuantMethod::Pq,
        }
    }

    pub(crate) fn node_count(&self) -> usize {
        match self {
            Self::Sq8(store) => store.node_count(),
            Self::Pq(store) => store.node_count(),
        }
    }

    pub(crate) fn stats(&self) -> QuantizationStats {
        match self {
            Self::Sq8(store) => store.stats(),
            Self::Pq(store) => store.stats(),
        }
    }

    #[allow(dead_code)]
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
        match self {
            Self::Sq8(store) => store.build_query_lut_into(query, metric, out),
            Self::Pq(store) => store.build_query_lut_into(query, metric, out),
        }
    }

    pub(crate) fn lut_sum(&self, lut: &[f32], node_idx: usize) -> Option<f32> {
        match self {
            Self::Sq8(store) => store.lut_sum(lut, node_idx),
            Self::Pq(store) => store.lut_sum(lut, node_idx),
        }
    }

    pub(crate) fn approx_norm(&self, node_idx: usize) -> Option<f32> {
        match self {
            Self::Sq8(store) => store.approx_norm(node_idx),
            Self::Pq(store) => store.approx_norm(node_idx),
        }
    }

    /// Return the PQ code bytes for `node_idx` when this store is PQ;
    /// `None` for SQ8 stores (SQ8 has no centroid-byte representation).
    pub(crate) fn pq_codes_for(&self, node_idx: usize) -> Option<&[u8]> {
        match self {
            Self::Sq8(_) => None,
            Self::Pq(store) => store.codes_for(node_idx),
        }
    }

    /// Encode `query` into byte codes for the polysemous Hamming pre-filter.
    /// `None` for SQ8 stores.
    pub(crate) fn pq_encode_query_codes(&self, query: &[f32]) -> Option<Box<[u8]>> {
        match self {
            Self::Sq8(_) => None,
            Self::Pq(store) => Some(store.encode_query_codes(query)),
        }
    }

    /// Whether the loaded PQ codebook was polysemous-permuted. `false` for
    /// SQ8 stores or PQ stores trained without polysemous.
    pub(crate) fn polysemous_trained(&self) -> bool {
        match self {
            Self::Sq8(_) => false,
            Self::Pq(store) => store.polysemous_trained(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OPQ_MAX_DIM, PqParams};
    use crate::VectorError;

    #[test]
    fn pq_default_for_dim_always_passes_validate_for_dim() {
        for dim in 1..=512 {
            let params = PqParams::default_for_dim(dim);
            params.validate_for_dim(dim).unwrap_or_else(|err| {
                panic!("default_for_dim({dim}) fails validate_for_dim: {err:?}")
            });
        }
    }

    #[test]
    fn pq_default_for_dim_17_uses_valid_divisor() {
        let params = PqParams::default_for_dim(17);

        assert_eq!(params.m_subspaces, 1);
        params.validate_for_dim(17).unwrap();
    }

    #[test]
    fn opq_default_respects_dim_cap() {
        assert!(PqParams::default_for_dim(OPQ_MAX_DIM).use_opq);
        assert!(!PqParams::default_for_dim(OPQ_MAX_DIM + 1).use_opq);
    }

    #[test]
    fn explicit_opq_above_dim_cap_is_rejected() {
        let params = PqParams {
            use_opq: true,
            ..PqParams::default_for_dim(OPQ_MAX_DIM + 1)
        };

        let err = params
            .validate_for_dim(OPQ_MAX_DIM + 1)
            .expect_err("explicit OPQ above the cap is invalid");

        assert!(matches!(err, VectorError::InvalidConfig { .. }));
    }
}
