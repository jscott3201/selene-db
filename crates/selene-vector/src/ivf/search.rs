//! IVF-PQ search with residual ADC scoring.

use roaring::RoaringBitmap;
use selene_core::NodeId;

use crate::hnsw::distance::dot_product;
use crate::{DistanceMetric, IvfConfig, VectorError};

use super::IvfIndex;

/// Search an IVF-PQ index for the top-`k` nearest neighbors.
///
/// Results are `(NodeId, score)` pairs sorted by score descending. The score
/// matches the HNSW higher-is-better convention.
pub fn search(
    index: &IvfIndex,
    query: &[f32],
    k: usize,
    n_probe: Option<u32>,
    config: &IvfConfig,
    filter: Option<&RoaringBitmap>,
) -> Result<Vec<(NodeId, f32)>, VectorError> {
    validate_query(query, config.dim)?;
    if k == 0 || !index.is_trained() {
        return Ok(Vec::new());
    }
    let probe_count = n_probe.unwrap_or(config.n_probe);
    if probe_count == 0 || probe_count > config.k_coarse {
        return Err(VectorError::IvfInvalidNProbe {
            n_probe: probe_count,
            k_coarse: config.k_coarse,
        });
    }
    let Some(coarse) = index.coarse_quantizer.as_deref() else {
        return Ok(Vec::new());
    };
    let Some(codebook) = index.residual_codebook.as_deref() else {
        return Ok(Vec::new());
    };
    let probes = coarse.nearest_probes(query, probe_count);
    let query_norm = dot_product(query, query).sqrt();
    let mut out = Vec::new();
    for centroid_id in probes {
        let Some(centroid) = coarse.centroid(centroid_id) else {
            continue;
        };
        let Some(list) = index.posting_lists.get(centroid_id as usize) else {
            continue;
        };
        let residual_query = match config.metric {
            DistanceMetric::L2 => query
                .iter()
                .zip(centroid)
                .map(|(left, right)| left - right)
                .collect::<Vec<_>>(),
            DistanceMetric::Dot | DistanceMetric::Cosine => query.to_vec(),
        };
        let lut = codebook.build_query_lut(&residual_query, config.metric);
        let centroid_dot = dot_product(query, centroid);
        for entry in &list.entries {
            let Some(lut_sum) = codebook.lut_sum_for_codes(&lut, &entry.codes) else {
                continue;
            };
            let score = match config.metric {
                DistanceMetric::L2 => -lut_sum.sqrt(),
                DistanceMetric::Dot => lut_sum + centroid_dot,
                DistanceMetric::Cosine => {
                    let Some(reconstructed_norm) = entry.reconstructed_norm else {
                        continue;
                    };
                    if query_norm == 0.0 || reconstructed_norm == 0.0 {
                        -1.0
                    } else {
                        ((lut_sum + centroid_dot) / (query_norm * reconstructed_norm)) - 1.0
                    }
                }
            };
            if passes_filter(entry.node_id, filter) {
                out.push((entry.node_id, score));
            }
        }
    }
    out.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    out.truncate(k);
    Ok(out)
}

fn validate_query(query: &[f32], dim: usize) -> Result<(), VectorError> {
    if query.len() != dim {
        return Err(VectorError::DimensionsLocked {
            expected: dim,
            observed: query.len(),
        });
    }
    for (index, value) in query.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(VectorError::NonFiniteQueryComponent { index, value });
        }
    }
    Ok(())
}

fn passes_filter(node_id: NodeId, filter: Option<&RoaringBitmap>) -> bool {
    let Some(bitmap) = filter else {
        return true;
    };
    let Ok(key) = u32::try_from(node_id.get()) else {
        return false;
    };
    bitmap.contains(key)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use selene_core::NodeId;

    use crate::{DistanceMetric, IvfConfig, PqParams};

    use super::super::RawVector;
    use super::super::train::train;
    use super::*;

    fn config(metric: DistanceMetric) -> IvfConfig {
        IvfConfig::with_params(
            2,
            2,
            1,
            metric,
            PqParams {
                m_subspaces: 1,
                k_centroids: 256,
                train_min_vectors: 256,
            },
            256,
        )
        .unwrap()
    }

    fn rows() -> Vec<RawVector> {
        (0..256)
            .map(|idx| RawVector {
                node_id: NodeId::new(idx + 1),
                vector: Arc::from([idx as f32, 0.0]),
            })
            .collect()
    }

    #[test]
    fn ivf_search_returns_valid_node_ids() {
        let config = config(DistanceMetric::L2);
        let trained = train(&rows(), &config).unwrap();
        let index = IvfIndex::trained(2, trained, 256);

        let results = search(&index, &[0.0, 0.0], 5, Some(2), &config, None).unwrap();

        assert!(results.iter().all(|(node_id, _)| node_id.get() > 0));
    }
}
