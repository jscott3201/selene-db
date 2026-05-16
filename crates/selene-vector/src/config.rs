//! HNSW provider configuration.

use serde::{Deserialize, Serialize};

use crate::{PqParams, QuantMethod, QuantizationConfig, VectorError};

/// Distance function used by the vector index.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[non_exhaustive]
pub enum DistanceMetric {
    /// Cosine similarity over dense f32 vectors.
    #[default]
    Cosine,
    /// Euclidean distance over dense f32 vectors.
    L2,
    /// Dot product over dense f32 vectors.
    Dot,
}

/// Build-time toggles for the HNSW diversity neighbor-selection heuristic.
///
/// `extend_candidates` widens the candidate pool by one layer-local hop
/// before applying the diversity filter. `keep_pruned_connections` fills
/// remaining neighbor slots from the rejected pile after diversity pruning.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NeighborSelectionConfig {
    /// Widen the candidate pool by one hop before diversity filtering.
    pub extend_candidates: bool,
    /// Backfill from rejected candidates when diversity selects fewer than M.
    pub keep_pruned_connections: bool,
}

impl Default for NeighborSelectionConfig {
    fn default() -> Self {
        Self {
            extend_candidates: false,
            keep_pruned_connections: true,
        }
    }
}

/// Configuration for [`crate::HnswProvider`].
// Eq dropped because `QuantizationConfig` transitively carries the f32
// `hamming_threshold_ratio`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HnswConfig {
    /// Vector dimensionality. Required; no default exists.
    pub dim: usize,
    /// Maximum neighbor count above layer 0.
    pub m: usize,
    /// Search width used while constructing the HNSW graph.
    pub ef_construction: usize,
    /// Default search width for query-time search.
    pub ef_search: usize,
    /// Distance metric used by this provider.
    pub metric: DistanceMetric,
    /// Optional quantized-search behavior.
    pub quantization: QuantizationConfig,
    /// Optional HNSW diversity neighbor-selection behavior.
    pub neighbor_selection: NeighborSelectionConfig,
}

impl HnswConfig {
    /// Donor-matched default neighbor count.
    #[must_use]
    pub const fn default_m() -> usize {
        16
    }

    /// Donor-matched default construction search width.
    #[must_use]
    pub const fn default_ef_construction() -> usize {
        200
    }

    /// Donor-matched default query search width.
    #[must_use]
    pub const fn default_ef_search() -> usize {
        50
    }

    /// Default vector distance metric.
    #[must_use]
    pub const fn default_metric() -> DistanceMetric {
        DistanceMetric::Cosine
    }

    /// Construct a config with donor-matched HNSW defaults for `dim`.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidConfig`] when `dim` is zero or exceeds
    /// `u16::MAX`, the dimensionality ceiling inherited from the graph body.
    pub fn new(dim: usize) -> Result<Self, VectorError> {
        Self::with_params(
            dim,
            Self::default_m(),
            Self::default_ef_construction(),
            Self::default_ef_search(),
            Self::default_metric(),
        )
    }

    /// Construct a config with explicit HNSW parameters.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidConfig`] when any parameter violates the
    /// HNSW configuration bounds.
    pub fn with_params(
        dim: usize,
        m: usize,
        ef_construction: usize,
        ef_search: usize,
        metric: DistanceMetric,
    ) -> Result<Self, VectorError> {
        let config = Self {
            dim,
            m,
            ef_construction,
            ef_search,
            metric,
            quantization: QuantizationConfig::default(),
            neighbor_selection: NeighborSelectionConfig::default(),
        };
        config.validate()?;
        Ok(config)
    }

    /// Return this config with explicit quantization behavior.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidConfig`] when the resulting config is
    /// invalid, including `rescore = true` while quantization is disabled.
    pub fn with_quantization(
        mut self,
        quantization: QuantizationConfig,
    ) -> Result<Self, VectorError> {
        self.quantization = quantization;
        self.validate()?;
        Ok(self)
    }

    /// Return this config with product quantization enabled.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidConfig`] or
    /// [`VectorError::PqDimensionNotDivisible`] when the resulting PQ config
    /// is invalid for this dimensionality.
    pub fn with_pq_quantization(mut self, pq: PqParams) -> Result<Self, VectorError> {
        self.quantization = QuantizationConfig {
            enabled: true,
            method: QuantMethod::Pq,
            rescore: false,
            pq: Some(pq),
        };
        self.validate()?;
        Ok(self)
    }

    /// Return this config with explicit HNSW neighbor-selection behavior.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidConfig`] when the resulting config is
    /// invalid.
    pub fn with_neighbor_selection(
        mut self,
        neighbor_selection: NeighborSelectionConfig,
    ) -> Result<Self, VectorError> {
        self.neighbor_selection = neighbor_selection;
        self.validate()?;
        Ok(self)
    }

    /// Validate this config.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidConfig`] when any field is outside the
    /// supported HNSW configuration bounds.
    pub fn validate(&self) -> Result<(), VectorError> {
        if self.dim == 0 {
            return Err(invalid_config("dim must be greater than zero"));
        }
        if self.dim > u16::MAX as usize {
            return Err(invalid_config("dim must be less than or equal to u16::MAX"));
        }
        if self.m < 2 {
            return Err(invalid_config("m must be at least 2"));
        }
        if self.m > u16::MAX as usize {
            return Err(invalid_config("m must be less than or equal to u16::MAX"));
        }
        if self.ef_construction < self.m {
            return Err(invalid_config(
                "ef_construction must be greater than or equal to m",
            ));
        }
        if self.ef_search == 0 {
            return Err(invalid_config("ef_search must be greater than zero"));
        }
        if self.quantization.rescore && !self.quantization.enabled {
            return Err(invalid_config(
                "quantization rescore requires quantization enabled",
            ));
        }
        if self.quantization.enabled && self.quantization.method == QuantMethod::Pq {
            PqParams::resolve(self.dim, self.quantization.pq).validate_for_dim(self.dim)?;
        }
        Ok(())
    }
}

/// Configuration for [`crate::IvfProvider`].
// Eq dropped because `PqParams` now carries the f32 `hamming_threshold_ratio`
// `hamming_threshold_ratio`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct IvfConfig {
    /// Vector dimensionality. Required; no default exists.
    pub dim: usize,
    /// Number of coarse centroids / posting lists.
    pub k_coarse: u32,
    /// Default number of posting lists scanned per query.
    pub n_probe: u32,
    /// Distance metric used by this provider.
    pub metric: DistanceMetric,
    /// Residual product-quantization parameters.
    pub pq: PqParams,
    /// Minimum vector count required before IVF-PQ training may run.
    pub training_min_vectors: usize,
}

impl IvfConfig {
    /// Fixed v1 coarse centroid default.
    pub const DEFAULT_K_COARSE: u32 = 256;

    /// Construct an IVF config with deterministic v1 defaults for `dim`.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidConfig`] when `dim` is zero or the derived
    /// defaults fail validation.
    pub fn new(dim: usize) -> Result<Self, VectorError> {
        Self::with_params(
            dim,
            Self::DEFAULT_K_COARSE,
            default_n_probe(Self::DEFAULT_K_COARSE),
            DistanceMetric::L2,
            PqParams::default_for_dim(dim),
            usize::max(
                (Self::DEFAULT_K_COARSE as usize).saturating_mul(39),
                PqParams::default_for_dim(dim).train_min_vectors,
            ),
        )
    }

    /// Construct an IVF config with explicit parameters.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidConfig`] when any field violates the IVF
    /// configuration bounds.
    pub fn with_params(
        dim: usize,
        k_coarse: u32,
        n_probe: u32,
        metric: DistanceMetric,
        pq: PqParams,
        training_min_vectors: usize,
    ) -> Result<Self, VectorError> {
        let config = Self {
            dim,
            k_coarse,
            n_probe,
            metric,
            pq,
            training_min_vectors,
        };
        config.validate()?;
        Ok(config)
    }

    /// Validate this config.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidConfig`] or
    /// [`VectorError::PqDimensionNotDivisible`] when a field is invalid.
    pub fn validate(&self) -> Result<(), VectorError> {
        if self.dim == 0 {
            return Err(invalid_config("IVF dim must be greater than zero"));
        }
        if self.dim > u16::MAX as usize {
            return Err(invalid_config(
                "IVF dim must be less than or equal to u16::MAX",
            ));
        }
        if self.k_coarse == 0 {
            return Err(invalid_config("IVF k_coarse must be greater than zero"));
        }
        if self.k_coarse > 65_536 {
            return Err(invalid_config("IVF k_coarse must be at most 65_536"));
        }
        if self.n_probe == 0 || self.n_probe > self.k_coarse {
            return Err(VectorError::IvfInvalidNProbe {
                n_probe: self.n_probe,
                k_coarse: self.k_coarse,
            });
        }
        self.pq.validate_for_dim(self.dim)?;
        let required = usize::max(self.k_coarse as usize, self.pq.train_min_vectors);
        if self.training_min_vectors < required {
            return Err(invalid_config(format!(
                "IVF training_min_vectors must be at least {required}, observed {}",
                self.training_min_vectors
            )));
        }
        Ok(())
    }
}

const fn default_n_probe(k_coarse: u32) -> u32 {
    let mut probe: u32 = 1;
    while probe.saturating_mul(probe) < k_coarse {
        probe += 1;
    }
    probe
}

fn invalid_config(reason: impl Into<String>) -> VectorError {
    VectorError::InvalidConfig {
        reason: reason.into(),
    }
}
