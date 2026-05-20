//! IVF-PQ search with residual ADC scoring.

use std::{cell::RefCell, cmp::Ordering, collections::BinaryHeap};

use roaring::RoaringBitmap;
use selene_core::NodeId;

use crate::hnsw::distance::dot_product;
use crate::{DistanceMetric, IvfConfig, VectorError};

use super::IvfIndex;

thread_local! {
    static IVF_SCRATCH: RefCell<IvfScratch> = const {
        RefCell::new(IvfScratch {
            residual_query: Vec::new(),
            probe_query_codes: Vec::new(),
        })
    };
}

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
    let mut scratch = IvfScratchLease::take();
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
        let scratch = scratch.as_mut();
        scratch.residual_query.clear();
        match effective_metric {
            DistanceMetric::L2 => scratch
                .residual_query
                .extend(query.iter().zip(centroid).map(|(left, right)| left - right)),
            DistanceMetric::Dot | DistanceMetric::Cosine => {
                scratch.residual_query.extend_from_slice(query);
            }
        }
        let residual_query = scratch.residual_query.as_slice();
        let lut = codebook.build_query_lut(residual_query, effective_metric);
        let centroid_dot = dot_product(query, centroid);
        // For L2 the per-probe residual changes, so we re-encode the query
        // codes against the possibly polysemous-permuted codebook for this
        // residual. Dot/Cosine disable this filter above because raw-query
        // codes and residual posting codes are different frames.
        let probe_query_codes: Option<&[u8]> = if hamming_filter_active {
            scratch.probe_query_codes.clear();
            codebook.encode_row(residual_query, &mut scratch.probe_query_codes);
            Some(scratch.probe_query_codes.as_slice())
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

#[derive(Default)]
struct IvfScratch {
    residual_query: Vec<f32>,
    probe_query_codes: Vec<u8>,
}

impl IvfScratch {
    fn clear(&mut self) {
        self.residual_query.clear();
        self.probe_query_codes.clear();
    }

    fn retained_capacity(&self) -> usize {
        self.residual_query
            .capacity()
            .saturating_add(self.probe_query_codes.capacity())
    }
}

struct IvfScratchLease {
    scratch: IvfScratch,
}

impl IvfScratchLease {
    fn take() -> Self {
        Self {
            scratch: IVF_SCRATCH.with(RefCell::take),
        }
    }

    fn as_mut(&mut self) -> &mut IvfScratch {
        &mut self.scratch
    }
}

impl Drop for IvfScratchLease {
    fn drop(&mut self) {
        let mut returned = std::mem::take(&mut self.scratch);
        returned.clear();
        let _ = IVF_SCRATCH.try_with(|cell| {
            let mut scratch = cell.borrow_mut();
            if returned.retained_capacity() > scratch.retained_capacity() {
                *scratch = returned;
            }
        });
    }
}

#[cfg(test)]
fn clear_ivf_scratch_for_test() {
    IVF_SCRATCH.with(|cell| *cell.borrow_mut() = IvfScratch::default());
}

#[cfg(test)]
fn ivf_scratch_capacity_for_test() -> (usize, usize) {
    IVF_SCRATCH.with(|cell| {
        let scratch = cell.borrow();
        (
            scratch.residual_query.capacity(),
            scratch.probe_query_codes.capacity(),
        )
    })
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

    #[test]
    fn bounded_heap_matches_legacy_sort_and_truncate() {
        let candidates = vec![
            (NodeId::new(9), 0.25),
            (NodeId::new(2), 1.0),
            (NodeId::new(7), -0.5),
            (NodeId::new(1), 1.0),
            (NodeId::new(4), 0.75),
            (NodeId::new(3), 0.75),
        ];

        for k in [1, 2, 3, 10] {
            let mut heap = BinaryHeap::with_capacity(k + 1);
            for (node_id, score) in candidates.iter().copied() {
                push_top_k(&mut heap, k, node_id, score);
            }
            let mut observed = heap
                .into_iter()
                .map(|entry| (entry.node_id, entry.score))
                .collect::<Vec<_>>();
            sort_results(&mut observed);

            let mut expected = candidates.clone();
            sort_results(&mut expected);
            expected.truncate(k);
            assert_eq!(observed, expected, "k={k}");
        }
    }

    #[test]
    fn bounded_heap_tie_break_keeps_lower_node_id() {
        let mut heap = BinaryHeap::with_capacity(3);
        for node_id in [NodeId::new(3), NodeId::new(1), NodeId::new(2)] {
            push_top_k(&mut heap, 2, node_id, 1.0);
        }
        let mut observed = heap
            .into_iter()
            .map(|entry| (entry.node_id, entry.score))
            .collect::<Vec<_>>();
        sort_results(&mut observed);

        assert_eq!(observed[0].0, NodeId::new(1));
        assert_eq!(observed[1].0, NodeId::new(2));
    }

    #[test]
    fn bounded_heap_skips_nan_scores() {
        let mut heap = BinaryHeap::with_capacity(3);
        push_top_k(&mut heap, 2, NodeId::new(1), f32::NAN);
        push_top_k(&mut heap, 2, NodeId::new(2), 0.5);

        let observed = heap
            .into_iter()
            .map(|entry| (entry.node_id, entry.score))
            .collect::<Vec<_>>();

        assert_eq!(observed, vec![(NodeId::new(2), 0.5)]);
    }

    #[test]
    fn scratch_lease_reentrant_take_does_not_panic() {
        clear_ivf_scratch_for_test();
        {
            let mut outer = IvfScratchLease::take();
            outer.scratch.residual_query.reserve(16);
            outer.scratch.probe_query_codes.reserve(8);
            {
                let mut inner = IvfScratchLease::take();
                inner.scratch.residual_query.reserve(4);
                inner.scratch.probe_query_codes.reserve(2);
            }
        }

        let (residual_capacity, codes_capacity) = ivf_scratch_capacity_for_test();
        assert!(residual_capacity >= 16);
        assert!(codes_capacity >= 8);
    }

    #[test]
    fn scratch_reuse_capacity_stays_bounded_across_searches() {
        clear_ivf_scratch_for_test();
        let config = config(DistanceMetric::L2);
        let trained = train(&rows(), &config).unwrap();
        let index = IvfIndex::trained(2, trained, 256);

        search(&index, &[0.0, 0.0], 5, Some(2), &config, None, None).unwrap();
        let (warm_residual, warm_codes) = ivf_scratch_capacity_for_test();
        for _ in 0..100 {
            search(&index, &[1.0, 0.0], 5, Some(2), &config, None, None).unwrap();
        }
        let (final_residual, final_codes) = ivf_scratch_capacity_for_test();

        assert!(final_residual <= warm_residual.saturating_mul(2).max(2));
        assert!(final_codes <= warm_codes.saturating_mul(2).max(2));
    }
}
