//! HNSW two-phase search with optional RoaringBitmap result pre-filter.

use std::cmp::Ordering;
use std::collections::HashSet;

use roaring::RoaringBitmap;
use selene_core::NodeId;

use super::distance::distance;
use super::{HnswGraph, HnswParams, InternalIndex};
use crate::VectorError;

/// A scored HNSW candidate shared by build and search paths.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    /// Provider-local index.
    pub(crate) idx: InternalIndex,
    /// Higher-is-better score for the active distance metric.
    pub(crate) score: f32,
}

impl Candidate {
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
    for layer in (1..=graph.max_layer()).rev() {
        current_entry = greedy_search_layer(graph, query, current_entry, layer, params);
    }

    let mut beam = beam_search_layer(
        graph,
        query,
        current_entry,
        ef.max(k),
        0,
        filter,
        None,
        params,
    );
    beam.sort_by(Candidate::cmp);
    beam.truncate(k);

    Ok(beam
        .into_iter()
        .filter_map(|candidate| {
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
    query: &[f32],
    entry: InternalIndex,
    ef: usize,
    layer: u8,
    filter: Option<&RoaringBitmap>,
    exclude: Option<InternalIndex>,
    params: &HnswParams,
) -> Vec<Candidate> {
    if ef == 0 {
        return Vec::new();
    }

    let mut visited = HashSet::new();
    let mut frontier = Vec::new();
    let mut result = Vec::new();
    push_candidate(
        graph,
        query,
        entry,
        exclude,
        params,
        &mut visited,
        &mut frontier,
    );

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

        if candidate_passes_filter(graph, candidate.idx, filter) {
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
                    query,
                    *neighbor_idx,
                    exclude,
                    params,
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
    query: &[f32],
    entry: InternalIndex,
    layer: u8,
    params: &HnswParams,
) -> InternalIndex {
    let mut current = entry;
    let mut current_score = graph
        .node_by_idx(current)
        .map_or(f32::NEG_INFINITY, |node| score(query, &node.vector, params));

    loop {
        let mut improved = false;
        let Some(neighbors) = graph.iter_layer_neighbors(current, layer) else {
            break;
        };
        for neighbor_idx in neighbors {
            let Some(neighbor) = graph.node_by_idx(*neighbor_idx) else {
                continue;
            };
            let neighbor_score = score(query, &neighbor.vector, params);
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
    -distance(params.metric, a, b)
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
    query: &[f32],
    idx: InternalIndex,
    exclude: Option<InternalIndex>,
    params: &HnswParams,
    visited: &mut HashSet<InternalIndex>,
    out: &mut Vec<Candidate>,
) {
    if Some(idx) == exclude || !visited.insert(idx) {
        return;
    }
    if let Some(node) = graph.node_by_idx(idx) {
        out.push(Candidate {
            idx,
            score: score(query, &node.vector, params),
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
