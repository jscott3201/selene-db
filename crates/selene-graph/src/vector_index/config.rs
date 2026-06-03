//! Vector-index configuration validation.

use selene_core::HnswIndexConfig;

use crate::{GraphError, GraphResult};

use super::VectorIndexKind;

/// Upper bound for HNSW `M` accepted by the native graph engine.
pub(crate) const MAX_HNSW_MAX_NEIGHBORS: u16 = 512;
/// Upper bound for HNSW construction beam width.
pub(crate) const MAX_HNSW_EF_CONSTRUCTION: u16 = 4096;

pub(crate) fn hnsw_config_for_kind(
    kind: VectorIndexKind,
    config: Option<HnswIndexConfig>,
) -> GraphResult<Option<HnswIndexConfig>> {
    match (kind.hnsw_metric(), config) {
        (None, None) => Ok(None),
        (None, Some(config)) => Err(invalid_config(
            config,
            "only HNSW vector indexes accept HNSW config",
        )),
        (Some(_), None) => Ok(Some(HnswIndexConfig::default())),
        (Some(_), Some(config)) => {
            validate_hnsw_config(config)?;
            Ok(Some(config))
        }
    }
}

pub(crate) fn validate_hnsw_config(config: HnswIndexConfig) -> GraphResult<()> {
    if config.max_neighbors == 0 {
        return Err(invalid_config(
            config,
            "max_neighbors must be greater than zero",
        ));
    }
    if config.max_neighbors > MAX_HNSW_MAX_NEIGHBORS {
        return Err(invalid_config(config, "max_neighbors exceeds engine cap"));
    }
    if config.ef_construction == 0 {
        return Err(invalid_config(
            config,
            "ef_construction must be greater than zero",
        ));
    }
    if config.ef_construction > MAX_HNSW_EF_CONSTRUCTION {
        return Err(invalid_config(config, "ef_construction exceeds engine cap"));
    }
    if config.ef_construction < config.max_neighbors {
        return Err(invalid_config(
            config,
            "ef_construction must be at least max_neighbors",
        ));
    }
    Ok(())
}

fn invalid_config(config: HnswIndexConfig, reason: &'static str) -> GraphError {
    GraphError::VectorIndexInvalidHnswConfig {
        max_neighbors: config.max_neighbors,
        ef_construction: config.ef_construction,
        reason,
    }
}
