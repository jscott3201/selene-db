//! IVF-PQ search with residual ADC scoring.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use roaring::RoaringBitmap;
use selene_core::NodeId;

use crate::hnsw::distance::dot_product;
use crate::{DistanceMetric, IvfConfig, VectorError};

use super::IvfIndex;

/// Search an IVF-PQ index for the top-`k` nearest neighbors.
///
/// Results are `(NodeId, score)` pairs sorted by score descending. The score
/// matches the HNSW higher-is-better convention. NaN scores are skipped.
pub fn search(
    index: &IvfIndex,
    query: &[f32],
    k: usize,
    n_probe: Option<u32>,
    config: &IvfConfig,
    filter: Option<&RoaringBitmap>,
    metric_override: Option<DistanceMetric>,
) -> Result<Vec<(NodeId, f32)>, VectorError> {
    validate_query(query, config.dim)?;
    if k == 0 || !index.is_trained() {
        return Ok(Vec::new());
    }
    let effective_metric = metric_override.unwrap_or(config.metric);
    if effective_metric == DistanceMetric::Cosine && config.metric != DistanceMetric::Cosine {
        return Err(VectorError::IvfMetricOverrideRequiresSideData {
            r#override: effective_metric,
            build: config.metric,
        });
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
    // Polysemous setup. The filter activates only when the embedder requested
    // it AND the residual codebook self-identifies as polysemous-trained.
    // Drift between the two is rejected at recovery time by
    // `validate_trained_codebook`, so reaching the search path means the two
    // flags MUST already agree; the `&&` here is defense in depth rather than
    // a drift-masking gate.
    let polysemous_active = config.pq.use_polysemous && codebook.polysemous_trained;
    // The stored IVF PQ codes are residual-frame codes. Dot and Cosine score
    // in the raw-query frame with centroid contribution folded back per entry,
    // so their query bytes are not comparable to stored residual bytes. Keep
    // the Hamming pre-filter only for L2 until a metric-specific residual-frame
    // query-code proof lands.
    let hamming_filter_active = polysemous_active && effective_metric == DistanceMetric::L2;
    let polysemous_threshold = if hamming_filter_active {
        config.pq.resolve_hamming_threshold()
    } else {
        0
    };
    let mut top = BinaryHeap::with_capacity(k.saturating_add(1));
    for centroid_id in probes {
        let Some(centroid) = coarse.centroid(centroid_id) else {
            continue;
        };
        let Some(list) = index.posting_lists.get(centroid_id as usize) else {
            continue;
        };
        let residual_query = match effective_metric {
            DistanceMetric::L2 => query
                .iter()
                .zip(centroid)
                .map(|(left, right)| left - right)
                .collect::<Vec<_>>(),
            DistanceMetric::Dot | DistanceMetric::Cosine => query.to_vec(),
        };
        let lut = codebook.build_query_lut(&residual_query, effective_metric);
        let centroid_dot = dot_product(query, centroid);
        // For L2 the per-probe residual changes, so we re-encode the query
        // codes against the possibly polysemous-permuted codebook for this
        // residual. Dot/Cosine disable this filter above because raw-query
        // codes and residual posting codes are different frames.
        let probe_query_codes: Option<Box<[u8]>> = if hamming_filter_active {
            Some(codebook_encode_row(codebook, &residual_query))
        } else {
            None
        };
        for entry in &list.entries {
            // Hamming pre-filter (V111): skip the LUT lookup entirely
            // when the stored polysemous code is too far from the query
            // in bit space. `continue` here directly converts the
            // filter-pass rate to throughput.
            if let Some(query_codes) = probe_query_codes.as_deref() {
                let hamming = query_codes
                    .iter()
                    .zip(entry.codes.iter())
                    .map(|(left, right)| (left ^ right).count_ones())
                    .sum::<u32>();
                if hamming > polysemous_threshold {
                    continue;
                }
            }
            let Some(lut_sum) = codebook.lut_sum_for_codes(&lut, &entry.codes) else {
                continue;
            };
            let score = match effective_metric {
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
                push_top_k(&mut top, k, entry.node_id, score);
            }
        }
    }
    let mut out = top
        .into_iter()
        .map(|entry| (entry.node_id, entry.score))
        .collect::<Vec<_>>();
    sort_results(&mut out);
    Ok(out)
}

#[derive(Clone, Copy, Debug)]
struct Worst {
    score: f32,
    node_id: NodeId,
}

impl Ord for Worst {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then_with(|| self.node_id.cmp(&other.node_id))
    }
}

impl PartialOrd for Worst {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Worst {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Worst {}

fn push_top_k(top: &mut BinaryHeap<Worst>, k: usize, node_id: NodeId, score: f32) {
    if score.is_nan() {
        return;
    }
    top.push(Worst { score, node_id });
    if top.len() > k {
        top.pop();
    }
}

fn sort_results(out: &mut [(NodeId, f32)]) {
    out.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
}

fn codebook_encode_row(codebook: &crate::quantize::PqCodebook, row: &[f32]) -> Box<[u8]> {
    let mut codes = Vec::with_capacity(codebook.m_subspaces as usize);
    codebook.encode_row(row, &mut codes);
    codes.into_boxed_slice()
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
                use_opq: false,
                use_polysemous: false,
                hamming_threshold_ratio: 0.5,
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

        let results = search(&index, &[0.0, 0.0], 5, Some(2), &config, None, None).unwrap();

        assert!(results.iter().all(|(node_id, _)| node_id.get() > 0));
    }
}
