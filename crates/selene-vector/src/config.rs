//! HNSW provider configuration.

use crate::VectorError;

/// Distance function used by the vector index.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum DistanceMetric {
    /// Cosine similarity over dense f32 vectors.
    #[default]
    Cosine,
    /// Squared Euclidean distance over dense f32 vectors.
    L2,
    /// Dot product over dense f32 vectors.
    Dot,
}

/// Configuration for [`crate::HnswProvider`].
#[derive(Clone, Debug, Eq, PartialEq)]
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
    /// `u16::MAX`, the dimensionality ceiling inherited from the donor graph
    /// body that later M8 briefs port.
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
    /// BRIEF-57 acceptance contract.
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
        };
        config.validate()?;
        Ok(config)
    }

    /// Validate this config.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidConfig`] when any field is outside the
    /// supported BRIEF-57 skeleton bounds.
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
        if self.ef_construction < self.m {
            return Err(invalid_config(
                "ef_construction must be greater than or equal to m",
            ));
        }
        if self.ef_search == 0 {
            return Err(invalid_config("ef_search must be greater than zero"));
        }
        Ok(())
    }
}

fn invalid_config(reason: impl Into<String>) -> VectorError {
    VectorError::InvalidConfig {
        reason: reason.into(),
    }
}
