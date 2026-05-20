//! Runtime HNSW build knobs derived from provider configuration.

use super::build::NeighborSelectionOpts;
use crate::{DistanceMetric, HnswConfig, NeighborSelectionConfig, QuantizationConfig};

/// Derived HNSW construction and search parameters.
#[derive(Clone, Debug)]
pub struct HnswParams {
    /// Max bidirectional connections per node on upper layers.
    pub m: usize,
    /// Max bidirectional connections per node on layer 0.
    pub m0: usize,
    /// Candidate width used while inserting into the graph.
    pub ef_construction: usize,
    /// Default candidate width used by future query-time search.
    pub ef_search: usize,
    /// Layer assignment probability factor, `1.0 / ln(m)`.
    pub level_factor: f64,
    /// Distance metric used by build-time candidate scoring.
    pub metric: DistanceMetric,
    /// Quantized-search behavior copied from provider configuration.
    pub(crate) quantization: QuantizationConfig,
    /// Neighbor-selection behavior copied from provider configuration.
    pub(crate) neighbor_selection: NeighborSelectionConfig,
}

impl HnswParams {
    /// Derive HNSW parameters from a validated provider config.
    #[must_use]
    pub fn from_config(config: &HnswConfig) -> Self {
        Self {
            m: config.m,
            m0: config.m * 2,
            ef_construction: config.ef_construction,
            ef_search: config.ef_search,
            level_factor: 1.0 / (config.m as f64).ln(),
            metric: config.metric,
            quantization: config.quantization,
            neighbor_selection: config.neighbor_selection,
        }
    }

    /// Derive HNSW parameters with a query-time metric override applied.
    #[must_use]
    pub fn from_config_with_metric_override(
        config: &HnswConfig,
        metric_override: Option<DistanceMetric>,
    ) -> Self {
        let mut params = Self::from_config(config);
        if let Some(metric) = metric_override {
            params.metric = metric;
        }
        params
    }

    /// Return the max neighbor count for `layer`.
    #[must_use]
    pub fn max_neighbors(&self, layer: u8) -> usize {
        if layer == 0 { self.m0 } else { self.m }
    }

    /// Return insertion-time neighbor-selection options.
    #[must_use]
    pub(crate) const fn neighbor_selection_opts(&self) -> NeighborSelectionOpts {
        NeighborSelectionOpts {
            extend_candidates: self.neighbor_selection.extend_candidates,
            keep_pruned_connections: self.neighbor_selection.keep_pruned_connections,
        }
    }
}
