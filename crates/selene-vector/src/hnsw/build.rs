//! HNSW insertion with metric-aware heuristic neighbor selection.

use std::sync::Arc;

use selene_core::NodeId;

use super::search::{Candidate, Scorer, beam_search_layer, greedy_search_layer, score};
use super::{HnswGraph, HnswNode, HnswParams, InternalIndex};
use crate::VectorError;

const MAX_LAYER: u8 = 32;

/// Sample an HNSW layer using exponential decay capped at 32.
#[must_use]
pub fn random_layer(rng: &mut fastrand::Rng, level_factor: f64) -> u8 {
    let r = rng.f64().max(f64::MIN_POSITIVE);
    let level = (-r.ln() * level_factor).floor() as u32;
    level.min(u32::from(MAX_LAYER)) as u8
}

/// Sample a layer from a freshly forked fastrand generator.
#[must_use]
pub fn random_layer_default(level_factor: f64) -> u8 {
    random_layer(&mut fastrand::Rng::new(), level_factor)
}

/// Insert one fresh node into `graph`.
///
/// # Errors
///
/// Returns a [`VectorError`] when dimensions mismatch, the node ID is invalid
/// or duplicated, or any vector component is not finite.
pub fn insert_node(
    graph: &mut HnswGraph,
    node_id: NodeId,
    vector: Arc<[f32]>,
    max_layer: u8,
    params: &HnswParams,
) -> Result<InternalIndex, VectorError> {
    if max_layer > MAX_LAYER {
        return Err(VectorError::MaxLayerExceedsCap {
            observed: max_layer,
            cap: MAX_LAYER,
        });
    }
    if graph.nodes.len() >= InternalIndex::MAX as usize {
        return Err(VectorError::InternalIndexExhausted {
            current: graph.nodes.len(),
        });
    }
    if vector.len() != graph.dimensions() {
        return Err(VectorError::DimensionsLocked {
            expected: graph.dimensions(),
            observed: vector.len(),
        });
    }
    if graph.idx_for(node_id).is_some() {
        return Err(VectorError::DuplicateNodeId { node_id });
    }
    validate_finite(node_id, &vector)?;

    let node = HnswNode::new(node_id, vector, max_layer)?;
    let new_idx = graph.nodes.len() as InternalIndex;
    let node_max_layer = node.max_layer;
    graph.nodes.push(node);
    graph.node_id_to_idx.insert(node_id, new_idx);

    if graph.nodes.len() == 1 {
        graph.entry_point = Some(new_idx);
        graph.max_layer = node_max_layer;
        return Ok(new_idx);
    }

    let Some(entry) = graph.entry_point else {
        graph.entry_point = Some(new_idx);
        graph.max_layer = graph.max_layer.max(node_max_layer);
        return Ok(new_idx);
    };

    let query_vec = Arc::clone(&graph.nodes[new_idx as usize].vector);
    let scorer = Scorer::f32(&query_vec, params.metric);
    let mut current_entry = entry;
    let top_layer = graph.max_layer;

    if top_layer > node_max_layer {
        for layer in ((node_max_layer + 1)..=top_layer).rev() {
            current_entry = greedy_search_layer(graph, current_entry, layer, &scorer);
        }
    }

    for layer in (0..=node_max_layer.min(top_layer)).rev() {
        let candidates = beam_search_layer(
            graph,
            current_entry,
            params.ef_construction,
            layer,
            None,
            Some(new_idx),
            &scorer,
        );
        if let Some(best) = candidates.first() {
            current_entry = best.idx;
        }

        let selected = select_neighbors_heuristic(
            graph,
            &query_vec,
            candidates,
            params.max_neighbors(layer),
            params,
        );
        graph.nodes[new_idx as usize].neighbors[layer as usize].clone_from(&selected);
        for neighbor_idx in selected {
            add_bidirectional_link(graph, neighbor_idx, new_idx, layer, params);
        }
    }

    if node_max_layer > graph.max_layer {
        graph.max_layer = node_max_layer;
        graph.entry_point = Some(new_idx);
    }

    Ok(new_idx)
}

pub(crate) fn validate_finite(node_id: NodeId, vector: &[f32]) -> Result<(), VectorError> {
    for (index, value) in vector.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(VectorError::NonFiniteVectorComponent {
                node_id,
                index,
                value,
            });
        }
    }
    Ok(())
}

pub(crate) fn select_neighbors_heuristic(
    graph: &HnswGraph,
    _target_vec: &[f32],
    mut candidates: Vec<Candidate>,
    m: usize,
    params: &HnswParams,
) -> Vec<InternalIndex> {
    candidates.sort_by(Candidate::cmp);
    let mut selected = Vec::with_capacity(m);

    for candidate in &candidates {
        if selected.len() >= m {
            break;
        }
        let Some(candidate_node) = graph.node_by_idx(candidate.idx) else {
            continue;
        };
        let too_close_to_existing = selected.iter().copied().any(|selected_idx| {
            let selected_vec = &graph.nodes[selected_idx as usize].vector;
            score(&candidate_node.vector, selected_vec, params) > candidate.score
        });
        if !too_close_to_existing {
            selected.push(candidate.idx);
        }
    }

    if selected.len() < m {
        for candidate in candidates {
            if selected.len() >= m {
                break;
            }
            if !selected.contains(&candidate.idx) {
                selected.push(candidate.idx);
            }
        }
    }

    selected
}

fn add_bidirectional_link(
    graph: &mut HnswGraph,
    from: InternalIndex,
    to: InternalIndex,
    layer: u8,
    params: &HnswParams,
) {
    {
        let neighbors = &mut graph.nodes[from as usize].neighbors[layer as usize];
        if !neighbors.contains(&to) {
            neighbors.push(to);
        }
    }
    if graph.nodes[from as usize].neighbors[layer as usize].len() > params.max_neighbors(layer) {
        prune_neighbors(graph, from, layer, params);
    }
}

fn prune_neighbors(graph: &mut HnswGraph, idx: InternalIndex, layer: u8, params: &HnswParams) {
    let target_vec = Arc::clone(&graph.nodes[idx as usize].vector);
    let old_neighbors = graph.nodes[idx as usize].neighbors[layer as usize].clone();
    let candidates = old_neighbors
        .iter()
        .copied()
        .filter_map(|neighbor_idx| {
            graph.node_by_idx(neighbor_idx).map(|neighbor| Candidate {
                idx: neighbor_idx,
                score: score(&target_vec, &neighbor.vector, params),
            })
        })
        .collect();
    let pruned = select_neighbors_heuristic(
        graph,
        &target_vec,
        candidates,
        params.max_neighbors(layer),
        params,
    );
    for removed in old_neighbors
        .iter()
        .copied()
        .filter(|old| !pruned.contains(old))
    {
        if let Some(neighbor) = graph.nodes.get_mut(removed as usize)
            && let Some(links) = neighbor.neighbors.get_mut(layer as usize)
        {
            links.retain(|linked| *linked != idx);
        }
    }
    graph.nodes[idx as usize].neighbors[layer as usize] = pruned;
}
