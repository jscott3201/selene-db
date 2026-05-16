//! HNSW two-phase search with optional RoaringBitmap result pre-filter.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};

use roaring::RoaringBitmap;
use selene_core::NodeId;

use super::distance::{distance, dot_product};
use super::{HnswGraph, HnswParams, InternalIndex};
use crate::DistanceMetric;
use crate::VectorError;
use crate::quantize::{PqParams, QuantizedStore};

thread_local! {
    static LUT_SCRATCH: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

/// A scored HNSW candidate shared by build and search paths.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    /// Provider-local index.
    pub(crate) idx: InternalIndex,
    /// Higher-is-better score for the active distance metric.
    pub(crate) score: f32,
    /// `false` when the candidate must route but cannot be yielded: the scorer
    /// returned `None` (e.g. polysemous Hamming-filter fail), or the physical
    /// slot is tombstoned. Non-admissible candidates may still expand the
    /// frontier during beam descent, but must not enter the result set and must
    /// not be resurrected by post-beam rescoring.
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

/// Heap wrapper whose `pop()` returns the best unexpanded candidate.
#[derive(Clone, Copy, Debug)]
struct FrontierCandidate(Candidate);

impl Ord for FrontierCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        Candidate::cmp(&other.0, &self.0)
    }
}

impl PartialOrd for FrontierCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for FrontierCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for FrontierCandidate {}

/// Heap wrapper whose `peek()` returns the worst kept result.
#[derive(Clone, Copy, Debug)]
struct ResultCandidate(Candidate);

impl Ord for ResultCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        Candidate::cmp(&self.0, &other.0)
    }
}

impl PartialOrd for ResultCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for ResultCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for ResultCandidate {}

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
    if graph.live_len() == 0 || k == 0 {
        return Ok(Vec::new());
    }

    let Some(mut current_entry) = graph.entry_point() else {
        return Ok(Vec::new());
    };
    if !graph.is_alive_idx(current_entry) {
        return Ok(Vec::new());
    }
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
    let mut results = beam
        .into_iter()
        .filter_map(|candidate| {
            if !candidate.admissible || !graph.is_alive_idx(candidate.idx) {
                return None;
            }
            graph
                .node_by_idx(candidate.idx)
                .map(|node| (node.node_id, candidate.score))
        })
        .collect::<Vec<_>>();
    results.truncate(k);
    Ok(results)
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
    let mut frontier = BinaryHeap::new();
    let mut result = BinaryHeap::new();
    if let Some(candidate) = make_candidate(graph, entry, exclude, scorer, &mut visited) {
        frontier.push(FrontierCandidate(candidate));
    }

    while let Some(FrontierCandidate(candidate)) = frontier.pop() {
        // Early termination: once result holds ef entries and the best
        // remaining frontier candidate is worse than the worst result, further
        // expansion cannot improve top-ef. This preserves the BRIEF-59
        // bounded-beam fix while allowing filtered nodes to route until the
        // admission set is full.
        //
        // BRIEF-69 cloud-Codex P1: non-admissible candidates carry a
        // `NEG_INFINITY` placeholder score (the scorer returned `None`),
        // which makes `Candidate::cmp` always say they're worse than every
        // admissible result entry. Without the admissibility gate below,
        // the loop would short-circuit before expanding a non-admissible
        // candidate's neighbors — and those neighbors are exactly where
        // the next admissible result entry might live. We only treat the
        // current candidate as a termination signal when it is itself
        // admissible (i.e. carries a real score the cmp can reason about).
        if result.len() >= ef
            && candidate.admissible
            && let Some(ResultCandidate(worst)) = result.peek()
            && Candidate::cmp(&candidate, worst).is_gt()
        {
            break;
        }

        // Admissibility gate: non-admissible candidates (e.g. polysemous
        // Hamming-filter failures or scorer-None nodes) may still expand the
        // frontier below, but must never enter `result` — otherwise they
        // leak into top-k whenever fewer than `ef` candidates are admissible.
        if candidate.admissible && candidate_passes_filter(graph, candidate.idx, filter) {
            result.push(ResultCandidate(candidate));
            if result.len() > ef {
                result.pop();
            }
        }

        if let Some(neighbors) = graph.iter_layer_neighbors(candidate.idx, layer) {
            for neighbor_idx in neighbors {
                if let Some(candidate) =
                    make_candidate(graph, *neighbor_idx, exclude, scorer, &mut visited)
                {
                    frontier.push(FrontierCandidate(candidate));
                }
            }
        }
    }

    let mut result = result
        .into_iter()
        .map(|candidate| candidate.0)
        .collect::<Vec<_>>();
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
            if !graph.is_alive_idx(*neighbor_idx) {
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
        /// Thread-local scratch leased for this scorer and returned on drop.
        lut: LutScratchLease,
        quantized: &'a QuantizedStore,
        /// Pre-computed polysemous query codes (BRIEF-69 §C.3). Populated
        /// when `params.quantization.pq.use_polysemous && quantized.polysemous_trained()`;
        /// `None` disables the Hamming pre-filter so non-polysemous stores
        /// behave exactly as they did before BRIEF-69.
        polysemous_query_codes: Option<Box<[u8]>>,
        /// Hamming distance cutoff (V112). When `polysemous_query_codes` is
        /// `Some`, candidates whose stored-vs-query Hamming exceeds this
        /// threshold are marked non-admissible. The admission bit added in
        /// Stage 1 delta 2 keeps them out of the result set even though
        /// they may still expand the frontier.
        polysemous_threshold: u32,
    },
}

pub(crate) struct LutScratchLease {
    lut: Vec<f32>,
}

impl LutScratchLease {
    fn build(quantized: &QuantizedStore, query: &[f32], metric: DistanceMetric) -> LutScratchLease {
        let mut lut = LUT_SCRATCH.with(RefCell::take);
        quantized.build_query_lut_into(query, metric, &mut lut);
        Self { lut }
    }

    fn as_slice(&self) -> &[f32] {
        &self.lut
    }
}

impl Drop for LutScratchLease {
    fn drop(&mut self) {
        let mut returned = std::mem::take(&mut self.lut);
        returned.clear();
        let _ = LUT_SCRATCH.try_with(|cell| {
            let mut scratch = cell.borrow_mut();
            if returned.capacity() > scratch.capacity() {
                *scratch = returned;
            }
        });
    }
}

#[cfg(test)]
fn clear_lut_scratch_for_test() {
    LUT_SCRATCH.with(|cell| cell.borrow_mut().clear());
}

#[cfg(test)]
fn lut_scratch_capacity_for_test() -> usize {
    LUT_SCRATCH.with(|cell| cell.borrow().capacity())
}

impl<'a> Scorer<'a> {
    pub(crate) fn f32(query: &'a [f32], metric: DistanceMetric) -> Self {
        Self::F32 { query, metric }
    }

    fn for_search(graph: &'a HnswGraph, query: &'a [f32], params: &HnswParams) -> Self {
        if params.quantization.enabled
            && let Some(quantized) = graph.quantized()
        {
            // The polysemous filter activates only when the embedder
            // requested it AND the loaded codebook self-identifies as
            // polysemous-trained (V107). The two are checked independently
            // so that recovery drift is caught by the QUNT/IPQB validator,
            // not silently masked by the scorer.
            //
            // PqParams::resolve mirrors the trainer (PqCodebook::train),
            // which falls back to PqParams::default_for_dim when the
            // embedder leaves `quantization.pq` as `None`. Without this
            // resolution the search-side filter would silently disable
            // itself whenever the trainer used defaults.
            let resolved_pq = PqParams::resolve(graph.dimensions(), params.quantization.pq);
            let (polysemous_query_codes, polysemous_threshold) =
                if resolved_pq.use_polysemous && quantized.polysemous_trained() {
                    (
                        quantized.pq_encode_query_codes(query),
                        resolved_pq.resolve_hamming_threshold(),
                    )
                } else {
                    (None, 0)
                };
            return Self::Asymmetric {
                query,
                metric: params.metric,
                query_norm: dot_product(query, query).sqrt(),
                lut: LutScratchLease::build(quantized, query, params.metric),
                quantized,
                polysemous_query_codes,
                polysemous_threshold,
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
                polysemous_query_codes,
                polysemous_threshold,
            } => {
                let node_idx = idx as usize;
                if node_idx >= quantized.node_count() {
                    return Some(score_for_metric(*metric, query, &node.vector));
                }
                // BRIEF-69 §C.3 Hamming pre-filter: when polysemous codes
                // are available AND the stored bytes are also polysemous,
                // skip ADC LUT lookup for candidates whose Hamming distance
                // from the query exceeds the threshold. Returning `None`
                // routes through candidate construction to a non-admissible
                // Candidate (BRIEF-69 Stage 1 delta 2 / V110) — the
                // candidate may still expand the frontier but cannot enter
                // the result set or be resurrected by rescore.
                if let Some(query_codes) = polysemous_query_codes
                    && let Some(stored_codes) = quantized.pq_codes_for(node_idx)
                {
                    let hamming = query_codes
                        .iter()
                        .zip(stored_codes)
                        .map(|(left, right)| (left ^ right).count_ones())
                        .sum::<u32>();
                    if hamming > *polysemous_threshold {
                        return None;
                    }
                }
                let lut_sum = quantized.lut_sum(lut.as_slice(), node_idx)?;
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

fn make_candidate(
    graph: &HnswGraph,
    idx: InternalIndex,
    exclude: Option<InternalIndex>,
    scorer: &Scorer<'_>,
    visited: &mut HashSet<InternalIndex>,
) -> Option<Candidate> {
    if Some(idx) == exclude || !visited.insert(idx) {
        return None;
    }
    if graph.node_by_idx(idx).is_some() {
        let scored = scorer.score(graph, idx);
        // Capture admissibility from `Some(_)` vs `None` *before* the
        // unwrap_or collapses both into NEG_INFINITY for ordering. Without
        // this bit, a polysemous Hamming-filter fail (or any future
        // `None`-returning scorer) would leak into top-k via the score
        // ordering path even though the scorer explicitly excluded it.
        return Some(Candidate {
            idx,
            score: scored.unwrap_or(f32::NEG_INFINITY),
            admissible: scored.is_some() && graph.is_alive_idx(idx),
        });
    }
    None
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn beam_search_layer_sorted_vec(
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
    if let Some(candidate) = make_candidate(graph, entry, exclude, scorer, &mut visited) {
        frontier.push(candidate);
    }

    while !frontier.is_empty() {
        frontier.sort_by(Candidate::cmp);
        let candidate = frontier.remove(0);

        if result.len() >= ef
            && candidate.admissible
            && let Some(worst) = result.last()
            && Candidate::cmp(&candidate, worst).is_gt()
        {
            break;
        }

        if candidate.admissible && candidate_passes_filter(graph, candidate.idx, filter) {
            result.push(candidate);
            result.sort_by(Candidate::cmp);
            if result.len() > ef {
                result.truncate(ef);
            }
        }

        if let Some(neighbors) = graph.iter_layer_neighbors(candidate.idx, layer) {
            for neighbor_idx in neighbors {
                if let Some(candidate) =
                    make_candidate(graph, *neighbor_idx, exclude, scorer, &mut visited)
                {
                    frontier.push(candidate);
                }
            }
        }
    }

    result.sort_by(Candidate::cmp);
    result
}

fn candidate_passes_filter(
    graph: &HnswGraph,
    idx: InternalIndex,
    filter: Option<&RoaringBitmap>,
) -> bool {
    graph
        .node_by_idx(idx)
        .is_some_and(|node| graph.is_alive_idx(idx) && passes_filter(node.node_id, filter))
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
    use std::thread;

    use selene_core::NodeId;

    use super::super::build::insert_node;
    use super::super::delete::tombstone_node;
    use super::*;
    use crate::quantize::QuantizedStoreSq8;
    use crate::{HnswConfig, HnswNode, QuantizationConfig};

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

    #[test]
    fn tombstoned_articulation_node_still_routes_beam() {
        let config =
            HnswConfig::with_params(2, 4, 16, 1, DistanceMetric::L2).expect("test config is valid");
        let params = HnswParams::from_config(&config);
        let mut graph = HnswGraph::empty(2);
        let mut entry = HnswNode::new(NodeId::new(1), Arc::from([0.0, 0.0]), 0).unwrap();
        let mut bridge = HnswNode::new(NodeId::new(2), Arc::from([100.0, 0.0]), 0).unwrap();
        let target = HnswNode::new(NodeId::new(3), Arc::from([2.0, 0.0]), 0).unwrap();
        entry.neighbors[0] = vec![1];
        bridge.neighbors[0] = vec![2];
        graph.nodes = vec![entry, bridge, target];
        graph.node_id_to_idx.insert(NodeId::new(1), 0);
        graph.node_id_to_idx.insert(NodeId::new(2), 1);
        graph.node_id_to_idx.insert(NodeId::new(3), 2);
        graph.mark_alive_idx(0);
        graph.mark_alive_idx(1);
        graph.mark_alive_idx(2);
        graph.entry_point = Some(0);

        tombstone_node(&mut graph, NodeId::new(2)).expect("bridge tombstone succeeds");
        let results = search(&graph, &[2.0, 0.0], 1, 1, &params, None).expect("search succeeds");

        assert_eq!(
            results.first().map(|(node_id, _)| *node_id),
            Some(NodeId::new(3))
        );
    }

    #[test]
    fn beam_search_heap_matches_sorted_vec_on_scalar_build() {
        let graph = beam_fixture_graph();
        let scorer = Scorer::f32(&[0.0, 0.0], DistanceMetric::L2);

        let heap = beam_search_layer(&graph, 0, 3, 0, None, None, &scorer);
        let sorted_vec = beam_search_layer_sorted_vec(&graph, 0, 3, 0, None, None, &scorer);

        assert_eq!(
            candidate_fingerprint(&heap),
            candidate_fingerprint(&sorted_vec)
        );
        for candidate in heap {
            assert!(graph.is_alive_idx(candidate.idx));
        }
    }

    #[test]
    fn beam_search_preserves_admissible_only_results() {
        let mut graph = graph_from_rows(&[
            (1, &[0.0, 0.0], 0),
            (2, &[100.0, 0.0], 0),
            (3, &[2.0, 0.0], 0),
        ]);
        graph.nodes[0].neighbors[0] = vec![1];
        graph.nodes[1].neighbors[0] = vec![2];
        tombstone_node(&mut graph, NodeId::new(2)).expect("bridge tombstone succeeds");
        let scorer = Scorer::f32(&[2.0, 0.0], DistanceMetric::L2);

        let results = beam_search_layer(&graph, 0, 1, 0, None, None, &scorer);

        assert_eq!(
            results
                .iter()
                .map(|candidate| candidate.idx)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert!(!results.iter().any(|candidate| candidate.idx == 1));
    }

    #[test]
    fn lut_scratch_reuse_does_not_leak_state_between_searches() {
        clear_lut_scratch_for_test();
        let rows = &[&[0.0, 0.0][..], &[8.0, 0.0][..], &[0.0, 8.0][..]];
        let (graph, params) = graph_with_quantized(
            DistanceMetric::L2,
            rows,
            QuantizationConfig {
                enabled: true,
                rescore: false,
                ..Default::default()
            },
        );

        let first = search(&graph, &[0.1, 0.0], 1, 4, &params, None).expect("search succeeds");
        assert_eq!(
            first.first().map(|(node_id, _)| *node_id),
            Some(NodeId::new(1))
        );
        let scratch_capacity = lut_scratch_capacity_for_test();
        assert!(scratch_capacity >= graph.dimensions() * 256);

        let second = search(&graph, &[7.9, 0.0], 1, 4, &params, None).expect("search succeeds");
        assert_eq!(
            second.first().map(|(node_id, _)| *node_id),
            Some(NodeId::new(2))
        );
        assert_eq!(lut_scratch_capacity_for_test(), scratch_capacity);

        let graph = Arc::new(graph);
        let params = Arc::new(params);
        let left_graph = Arc::clone(&graph);
        let left_params = Arc::clone(&params);
        let left = thread::spawn(move || {
            search(&left_graph, &[0.1, 0.0], 1, 4, &left_params, None).expect("search succeeds")
        });
        let right_graph = Arc::clone(&graph);
        let right_params = Arc::clone(&params);
        let right = thread::spawn(move || {
            search(&right_graph, &[7.9, 0.0], 1, 4, &right_params, None).expect("search succeeds")
        });

        assert_eq!(left.join().expect("thread joins"), first);
        assert_eq!(right.join().expect("thread joins"), second);
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

    fn beam_fixture_graph() -> HnswGraph {
        let mut graph = graph_from_rows(&[
            (1, &[0.0, 0.0], 0),
            (2, &[1.0, 0.0], 0),
            (3, &[0.0, 2.0], 0),
            (4, &[0.5, 0.0], 0),
            (5, &[3.0, 0.0], 0),
        ]);
        graph.nodes[0].neighbors[0] = vec![1, 2];
        graph.nodes[1].neighbors[0] = vec![3, 4];
        graph.nodes[2].neighbors[0] = vec![4];
        graph.nodes[3].neighbors[0] = vec![4];
        graph
    }

    fn graph_from_rows(rows: &[(u64, &[f32], u8)]) -> HnswGraph {
        let dimensions = rows.first().map_or(2, |(_, vector, _)| vector.len()) as u16;
        let mut graph = HnswGraph::empty(dimensions);
        for (idx, (raw, vector, layer)) in rows.iter().enumerate() {
            let node_id = NodeId::new(*raw);
            graph.nodes.push(
                HnswNode::new(node_id, Arc::from(*vector), *layer).expect("test node is valid"),
            );
            graph.mark_alive_idx(idx as InternalIndex);
            graph.node_id_to_idx.insert(node_id, idx as InternalIndex);
        }
        graph.entry_point = (!graph.nodes.is_empty()).then_some(0);
        graph.max_layer = rows.iter().map(|(_, _, layer)| *layer).max().unwrap_or(0);
        graph
    }

    fn candidate_fingerprint(candidates: &[Candidate]) -> Vec<(InternalIndex, u32, bool)> {
        candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.idx,
                    candidate.score.to_bits(),
                    candidate.admissible,
                )
            })
            .collect()
    }
}
