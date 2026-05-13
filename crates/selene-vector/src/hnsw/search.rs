//! HNSW two-phase search with optional RoaringBitmap result pre-filter.

use std::cmp::Ordering;
use std::collections::HashSet;

use roaring::RoaringBitmap;
use selene_core::NodeId;

use super::distance::{distance, dot_product};
use super::{HnswGraph, HnswParams, InternalIndex};
use crate::DistanceMetric;
use crate::VectorError;
use crate::quantize::QuantizedStore;

/// A scored HNSW candidate shared by build and search paths.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    /// Provider-local index.
    pub(crate) idx: InternalIndex,
    /// Higher-is-better score for the active distance metric.
    pub(crate) score: f32,
    /// `false` when the candidate's score is a `NEG_INFINITY` placeholder
    /// (scorer returned `None`, e.g. polysemous Hamming-filter fail).
    /// Non-admissible candidates may still expand the frontier during beam
    /// descent, but must not enter the result set and must not be resurrected
    /// by post-beam rescoring.
    pub(crate) admissible: bool,
}

impl Candidate {
    /// Construct a candidate that is admissible to the result set.
    pub(crate) const fn admissible(idx: InternalIndex, score: f32) -> Self {
        Self {
            idx,
            score,
            admissible: true,
        }
    }

    /// Sort by score descending, then provider-local index ascending.
    pub(crate) fn cmp(left: &Self, right: &Self) -> Ordering {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.idx.cmp(&right.idx))
    }
}

/// Search an immutable HNSW graph snapshot for the top-`k` nearest neighbors.
///
/// Results are `(NodeId, score)` pairs sorted by score descending. The score is
/// `-distance(metric, query, candidate.vector)`, so higher is always better.
///
/// `ef` is the beam width. Values below `k` are widened to `k`. When `filter`
/// is present, all discovered nodes may route the search, but only nodes whose
/// raw [`NodeId`] value converts to `u32` and appears in the bitmap are
/// admitted to the result set.
///
/// # Errors
///
/// Returns [`VectorError::DimensionsLocked`] when `query` has the wrong
/// dimensionality, or [`VectorError::NonFiniteQueryComponent`] when a query
/// component is NaN or infinite.
pub fn search(
    graph: &HnswGraph,
    query: &[f32],
    k: usize,
    ef: usize,
    params: &HnswParams,
    filter: Option<&RoaringBitmap>,
) -> Result<Vec<(NodeId, f32)>, VectorError> {
    if query.len() != graph.dimensions() {
        return Err(VectorError::DimensionsLocked {
            expected: graph.dimensions(),
            observed: query.len(),
        });
    }
    validate_query(query)?;
    if graph.is_empty() || k == 0 {
        return Ok(Vec::new());
    }

    let Some(mut current_entry) = graph.entry_point() else {
        return Ok(Vec::new());
    };
    let scorer = Scorer::for_search(graph, query, params);
    for layer in (1..=graph.max_layer()).rev() {
        current_entry = greedy_search_layer(graph, current_entry, layer, &scorer);
    }

    let mut beam = beam_search_layer(graph, current_entry, ef.max(k), 0, filter, None, &scorer);
    beam.sort_by(Candidate::cmp);
    if params.quantization.rescore && scorer.is_asymmetric() {
        for candidate in &mut beam {
            // Rescore only admissible candidates; a non-admissible candidate
            // (polysemous Hamming-filter fail) must stay out of the result.
            if !candidate.admissible {
                continue;
            }
            if let Some(node) = graph.node_by_idx(candidate.idx) {
                candidate.score = score(query, &node.vector, params);
            }
        }
        beam.sort_by(Candidate::cmp);
    }
    beam.truncate(k);

    Ok(beam
        .into_iter()
        .filter_map(|candidate| {
            if !candidate.admissible {
                return None;
            }
            graph
                .node_by_idx(candidate.idx)
                .map(|node| (node.node_id, candidate.score))
        })
        .collect())
}

/// Search one HNSW layer with an `ef`-bounded beam.
#[allow(clippy::too_many_arguments)]
pub(crate) fn beam_search_layer(
    graph: &HnswGraph,
    entry: InternalIndex,
    ef: usize,
    layer: u8,
    filter: Option<&RoaringBitmap>,
    exclude: Option<InternalIndex>,
    scorer: &Scorer<'_>,
) -> Vec<Candidate> {
    if ef == 0 {
        return Vec::new();
    }

    let mut visited = HashSet::new();
    let mut frontier = Vec::new();
    let mut result = Vec::new();
    push_candidate(graph, entry, exclude, scorer, &mut visited, &mut frontier);

    while !frontier.is_empty() {
        frontier.sort_by(Candidate::cmp);
        let candidate = frontier.remove(0);

        // Early termination: once result holds ef entries and the best
        // remaining frontier candidate is worse than the worst result, further
        // expansion cannot improve top-ef. This preserves the BRIEF-59
        // bounded-beam fix while allowing filtered nodes to route until the
        // admission set is full.
        if result.len() >= ef
            && let Some(worst) = result.last()
            && Candidate::cmp(&candidate, worst).is_gt()
        {
            break;
        }

        // Admissibility gate: non-admissible candidates (e.g. polysemous
        // Hamming-filter failures or scorer-None nodes) may still expand the
        // frontier below, but must never enter `result` — otherwise they
        // leak into top-k whenever fewer than `ef` candidates are admissible.
        if candidate.admissible && candidate_passes_filter(graph, candidate.idx, filter) {
            result.push(candidate);
            result.sort_by(Candidate::cmp);
            if result.len() > ef {
                result.truncate(ef);
            }
        }

        if let Some(neighbors) = graph.iter_layer_neighbors(candidate.idx, layer) {
            for neighbor_idx in neighbors {
                push_candidate(
                    graph,
                    *neighbor_idx,
                    exclude,
                    scorer,
                    &mut visited,
                    &mut frontier,
                );
            }
        }
    }

    result.sort_by(Candidate::cmp);
    result
}

/// Greedily descend one layer from `entry`.
pub(crate) fn greedy_search_layer(
    graph: &HnswGraph,
    entry: InternalIndex,
    layer: u8,
    scorer: &Scorer<'_>,
) -> InternalIndex {
    let mut current = entry;
    let mut current_score = scorer.score(graph, current).unwrap_or(f32::NEG_INFINITY);

    loop {
        let mut improved = false;
        let Some(neighbors) = graph.iter_layer_neighbors(current, layer) else {
            break;
        };
        for neighbor_idx in neighbors {
            if graph.node_by_idx(*neighbor_idx).is_none() {
                continue;
            }
            let neighbor_score = scorer
                .score(graph, *neighbor_idx)
                .unwrap_or(f32::NEG_INFINITY);
            let better = neighbor_score > current_score
                || (neighbor_score == current_score && *neighbor_idx < current);
            if better {
                current = *neighbor_idx;
                current_score = neighbor_score;
                improved = true;
            }
        }
        if !improved {
            return current;
        }
    }
    current
}

/// Compute the higher-is-better score used by HNSW candidate ordering.
pub(crate) fn score(a: &[f32], b: &[f32], params: &HnswParams) -> f32 {
    score_for_metric(params.metric, a, b)
}

/// Candidate scorer shared by every search walk step.
pub(crate) enum Scorer<'a> {
    /// Existing f32 kernel. Build paths always use this scorer.
    F32 {
        query: &'a [f32],
        metric: DistanceMetric,
    },
    /// Asymmetric quantized scorer with f32 fallback for post-snapshot nodes.
    Asymmetric {
        query: &'a [f32],
        metric: DistanceMetric,
        query_norm: f32,
        lut: Vec<f32>,
        quantized: &'a QuantizedStore,
    },
}

impl<'a> Scorer<'a> {
    pub(crate) fn f32(query: &'a [f32], metric: DistanceMetric) -> Self {
        Self::F32 { query, metric }
    }

    fn for_search(graph: &'a HnswGraph, query: &'a [f32], params: &HnswParams) -> Self {
        if params.quantization.enabled
            && let Some(quantized) = graph.quantized()
        {
            return Self::Asymmetric {
                query,
                metric: params.metric,
                query_norm: dot_product(query, query).sqrt(),
                lut: quantized.build_query_lut(query, params.metric),
                quantized,
            };
        }
        Self::f32(query, params.metric)
    }

    fn is_asymmetric(&self) -> bool {
        matches!(self, Self::Asymmetric { .. })
    }

    pub(crate) fn score(&self, graph: &HnswGraph, idx: InternalIndex) -> Option<f32> {
        let node = graph.node_by_idx(idx)?;
        match self {
            Self::F32 { query, metric } => Some(score_for_metric(*metric, query, &node.vector)),
            Self::Asymmetric {
                query,
                metric,
                query_norm,
                lut,
                quantized,
            } => {
                let node_idx = idx as usize;
                if node_idx >= quantized.node_count() {
                    return Some(score_for_metric(*metric, query, &node.vector));
                }
                let lut_sum = quantized.lut_sum(lut, node_idx)?;
                // §O.New-2: keep quantized and f32 scores on the same scale.
                Some(match metric {
                    DistanceMetric::L2 => -lut_sum.sqrt(),
                    DistanceMetric::Dot => lut_sum,
                    DistanceMetric::Cosine => {
                        let Some(approx_norm) = quantized.approx_norm(node_idx) else {
                            return Some(score_for_metric(*metric, query, &node.vector));
                        };
                        if *query_norm == 0.0 || approx_norm == 0.0 {
                            score_for_metric(*metric, query, &node.vector)
                        } else {
                            (lut_sum / (*query_norm * approx_norm)) - 1.0
                        }
                    }
                })
            }
        }
    }
}

fn score_for_metric(metric: DistanceMetric, a: &[f32], b: &[f32]) -> f32 {
    -distance(metric, a, b)
}

fn validate_query(query: &[f32]) -> Result<(), VectorError> {
    for (index, value) in query.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(VectorError::NonFiniteQueryComponent { index, value });
        }
    }
    Ok(())
}

fn push_candidate(
    graph: &HnswGraph,
    idx: InternalIndex,
    exclude: Option<InternalIndex>,
    scorer: &Scorer<'_>,
    visited: &mut HashSet<InternalIndex>,
    out: &mut Vec<Candidate>,
) {
    if Some(idx) == exclude || !visited.insert(idx) {
        return;
    }
    if graph.node_by_idx(idx).is_some() {
        let scored = scorer.score(graph, idx);
        // Capture admissibility from `Some(_)` vs `None` *before* the
        // unwrap_or collapses both into NEG_INFINITY for ordering. Without
        // this bit, a polysemous Hamming-filter fail (or any future
        // `None`-returning scorer) would leak into top-k via the score
        // ordering path even though the scorer explicitly excluded it.
        out.push(Candidate {
            idx,
            score: scored.unwrap_or(f32::NEG_INFINITY),
            admissible: scored.is_some(),
        });
    }
}

fn candidate_passes_filter(
    graph: &HnswGraph,
    idx: InternalIndex,
    filter: Option<&RoaringBitmap>,
) -> bool {
    graph
        .node_by_idx(idx)
        .is_some_and(|node| passes_filter(node.node_id, filter))
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

    use super::super::build::insert_node;
    use super::*;
    use crate::quantize::QuantizedStoreSq8;
    use crate::{HnswConfig, QuantizationConfig};

    #[test]
    fn lut_l2_score_matches_neg_sqrt_scale() {
        let (graph, params) = graph_with_quantized(
            DistanceMetric::L2,
            &[&[3.0, 4.0]],
            QuantizationConfig {
                enabled: true,
                rescore: false,
                ..Default::default()
            },
        );
        let scorer = Scorer::for_search(&graph, &[0.0, 0.0], &params);

        let observed = scorer.score(&graph, 0).expect("node scores");

        assert!((observed - -5.0).abs() <= 1e-6);
    }

    #[test]
    fn cosine_uses_approx_norms_cache() {
        let (graph, params) = graph_with_quantized(
            DistanceMetric::Cosine,
            &[&[3.0, 4.0]],
            QuantizationConfig {
                enabled: true,
                rescore: false,
                ..Default::default()
            },
        );
        let scorer = Scorer::for_search(&graph, &[3.0, 4.0], &params);
        let zero_scorer = Scorer::for_search(&graph, &[0.0, 0.0], &params);

        let exact = scorer.score(&graph, 0).expect("node scores");
        let zero = zero_scorer.score(&graph, 0).expect("zero query scores");

        assert!((exact - 0.0).abs() <= 1e-6);
        assert_eq!(
            zero,
            score_for_metric(DistanceMetric::Cosine, &[0.0, 0.0], &[3.0, 4.0])
        );
    }

    fn graph_with_quantized(
        metric: DistanceMetric,
        rows: &[&[f32]],
        quantization: QuantizationConfig,
    ) -> (HnswGraph, HnswParams) {
        let config = HnswConfig::with_params(rows[0].len(), 16, 200, 50, metric)
            .unwrap()
            .with_quantization(quantization)
            .unwrap();
        let params = HnswParams::from_config(&config);
        let mut graph = HnswGraph::empty(rows[0].len() as u16);
        for (offset, row) in rows.iter().enumerate() {
            insert_node(
                &mut graph,
                NodeId::new((offset + 1) as u64),
                Arc::from(*row),
                0,
                &params,
            )
            .unwrap();
        }
        let store = Arc::new(QuantizedStore::Sq8(
            QuantizedStoreSq8::build(graph.len(), graph.dimensions(), rows.iter().copied())
                .unwrap(),
        ));
        (graph.clone_with_quantized(Some(store)), params)
    }
}
