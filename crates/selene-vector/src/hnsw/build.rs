//! HNSW insertion with metric-aware heuristic neighbor selection.

use std::collections::HashSet;
use std::sync::Arc;

use selene_core::NodeId;

use super::search::{Candidate, Scorer, beam_search_layer, greedy_search_layer, score};
use super::{HnswGraph, HnswNode, HnswParams, InternalIndex};
use crate::VectorError;

const MAX_LAYER: u8 = 32;

/// Runtime toggles for HNSW neighbor selection.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NeighborSelectionOpts {
    /// Widen the candidate pool by one layer-local hop before pruning.
    pub(crate) extend_candidates: bool,
    /// Fill remaining slots from the diversity-pruned rejected pile.
    pub(crate) keep_pruned_connections: bool,
}

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
    graph.mark_alive_idx(new_idx);
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

    let opts = params.neighbor_selection_opts();
    for layer in (0..=node_max_layer.min(top_layer)).rev() {
        let mut candidates = beam_search_layer(
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
        if opts.extend_candidates {
            candidates = extend_candidate_pool(
                graph,
                &candidates,
                layer,
                &query_vec,
                params,
                params.ef_construction,
            );
        }

        let selected = select_neighbors_heuristic(
            graph,
            candidates,
            params.max_neighbors(layer),
            params,
            opts,
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
    mut candidates: Vec<Candidate>,
    m: usize,
    params: &HnswParams,
    opts: NeighborSelectionOpts,
) -> Vec<InternalIndex> {
    candidates.sort_by(Candidate::cmp);
    let mut selected = Vec::with_capacity(m);
    let mut rejected = Vec::new();
    let mut seen = HashSet::new();

    for candidate in &candidates {
        if selected.len() >= m {
            break;
        }
        if !seen.insert(candidate.idx) {
            continue;
        }
        if !graph.is_alive_idx(candidate.idx) {
            continue;
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
        } else {
            rejected.push(*candidate);
        }
    }

    if opts.keep_pruned_connections && selected.len() < m {
        for candidate in rejected {
            if selected.len() >= m {
                break;
            }
            if graph.is_alive_idx(candidate.idx) && !selected.contains(&candidate.idx) {
                selected.push(candidate.idx);
            }
        }
    }

    selected
}

/// Expand a candidate pool by one layer-local hop before diversity pruning.
pub(crate) fn extend_candidate_pool(
    graph: &HnswGraph,
    seeds: &[Candidate],
    layer: u8,
    target: &[f32],
    params: &HnswParams,
    cap: usize,
) -> Vec<Candidate> {
    if seeds.is_empty() || cap == 0 {
        return Vec::new();
    }

    let mut seen = HashSet::new();
    let mut expanded = Vec::with_capacity(seeds.len().min(cap));

    for seed in seeds {
        if seen.insert(seed.idx) && graph.is_alive_idx(seed.idx) {
            expanded.push(*seed);
        }
    }

    for seed in seeds {
        let Some(neighbors) = graph.iter_layer_neighbors(seed.idx, layer) else {
            continue;
        };
        for neighbor_idx in neighbors {
            if !seen.insert(*neighbor_idx) {
                continue;
            }
            if graph.is_alive_idx(*neighbor_idx)
                && let Some(neighbor) = graph.node_by_idx(*neighbor_idx)
            {
                expanded.push(Candidate::admissible(
                    *neighbor_idx,
                    score(target, &neighbor.vector, params),
                ));
            }
        }
    }

    expanded.sort_by(Candidate::cmp);
    expanded.truncate(cap);
    expanded
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
            if !graph.is_alive_idx(neighbor_idx) {
                return None;
            }
            graph.node_by_idx(neighbor_idx).map(|neighbor| {
                Candidate::admissible(neighbor_idx, score(&target_vec, &neighbor.vector, params))
            })
        })
        .collect();
    let pruned = select_neighbors_heuristic(
        graph,
        candidates,
        params.max_neighbors(layer),
        params,
        NeighborSelectionOpts {
            extend_candidates: false,
            keep_pruned_connections: params.neighbor_selection_opts().keep_pruned_connections,
        },
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

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::{DistanceMetric, HnswConfig};

    #[test]
    fn select_returns_at_most_m_neighbors() {
        let graph = graph(&[
            (1, &[1.0, 0.0], 0),
            (2, &[2.0, 0.0], 0),
            (3, &[3.0, 0.0], 0),
        ]);
        let selected = select_neighbors_heuristic(
            &graph,
            candidates(&graph, &[0.0, 0.0]),
            2,
            &params(),
            opts(true),
        );

        assert!(selected.len() <= 2);
    }

    #[test]
    fn select_admits_top_candidate_unconditionally() {
        let graph = graph(&[(1, &[1.0, 0.0], 0), (2, &[2.0, 0.0], 0)]);
        let selected = select_neighbors_heuristic(
            &graph,
            candidates(&graph, &[0.0, 0.0]),
            2,
            &params(),
            opts(false),
        );

        assert_eq!(selected.first().copied(), Some(0));
    }

    #[test]
    fn select_rejects_clustered_dups_when_diversity_filter_active() {
        let graph = graph(&[(1, &[1.0, 0.0], 0), (2, &[1.1, 0.0], 0)]);
        let selected = select_neighbors_heuristic(
            &graph,
            candidates(&graph, &[0.0, 0.0]),
            2,
            &params(),
            opts(false),
        );

        assert_eq!(selected, vec![0]);
    }

    #[test]
    fn select_falls_back_when_keep_pruned_on() {
        let graph = graph(&[(1, &[1.0, 0.0], 0), (2, &[1.1, 0.0], 0)]);
        let selected = select_neighbors_heuristic(
            &graph,
            candidates(&graph, &[0.0, 0.0]),
            2,
            &params(),
            opts(true),
        );

        assert_eq!(selected, vec![0, 1]);
    }

    #[test]
    fn select_returns_sub_m_when_keep_pruned_off() {
        let graph = graph(&[(1, &[1.0, 0.0], 0), (2, &[1.1, 0.0], 0)]);
        let selected = select_neighbors_heuristic(
            &graph,
            candidates(&graph, &[0.0, 0.0]),
            2,
            &params(),
            opts(false),
        );

        assert_eq!(selected.len(), 1);
    }

    #[test]
    fn select_backfills_only_from_rejected_pile_not_raw_candidates() {
        let graph = graph(&[(1, &[1.0, 0.0], 0), (2, &[1.1, 0.0], 0)]);
        let mut candidates = candidates(&graph, &[0.0, 0.0]);
        candidates.push(Candidate::admissible(99, f32::INFINITY));
        let selected = select_neighbors_heuristic(&graph, candidates, 2, &params(), opts(true));

        assert_eq!(selected, vec![0, 1]);
    }

    #[test]
    fn select_never_returns_invalid_candidate_index() {
        let graph = graph(&[(1, &[1.0, 0.0], 0)]);
        let selected = select_neighbors_heuristic(
            &graph,
            vec![
                Candidate::admissible(99, f32::INFINITY),
                Candidate::admissible(0, 0.0),
            ],
            2,
            &params(),
            opts(true),
        );

        assert_eq!(selected, vec![0]);
    }

    #[test]
    fn extend_pool_retains_seeds_once_and_dedupes() {
        let mut graph = graph(&[
            (1, &[1.0, 0.0], 0),
            (2, &[2.0, 0.0], 0),
            (3, &[3.0, 0.0], 0),
        ]);
        graph.nodes[0].neighbors[0] = vec![1, 2];
        graph.nodes[1].neighbors[0] = vec![2];
        let seeds = vec![
            Candidate::admissible(0, -1.0),
            Candidate::admissible(1, -2.0),
        ];

        let expanded = extend_candidate_pool(&graph, &seeds, 0, &[0.0, 0.0], &params(), 8);
        let mut ids = expanded
            .iter()
            .map(|candidate| candidate.idx)
            .collect::<Vec<_>>();
        ids.sort_unstable();

        assert_eq!(ids, vec![0, 1, 2]);
    }

    #[test]
    fn extend_pool_respects_cap() {
        let mut graph = graph(&[
            (1, &[1.0, 0.0], 0),
            (2, &[2.0, 0.0], 0),
            (3, &[3.0, 0.0], 0),
        ]);
        graph.nodes[0].neighbors[0] = vec![1, 2];

        let expanded = extend_candidate_pool(
            &graph,
            &[Candidate::admissible(0, -1.0)],
            0,
            &[0.0, 0.0],
            &params(),
            2,
        );

        assert_eq!(expanded.len(), 2);
    }

    #[test]
    fn extend_pool_is_layer_local() {
        let mut graph = graph(&[
            (1, &[1.0, 0.0], 1),
            (2, &[2.0, 0.0], 0),
            (3, &[3.0, 0.0], 1),
        ]);
        graph.nodes[0].neighbors[0] = vec![1];
        graph.nodes[0].neighbors[1] = vec![2];

        let expanded = extend_candidate_pool(
            &graph,
            &[Candidate::admissible(0, -1.0)],
            1,
            &[0.0, 0.0],
            &params(),
            8,
        );
        let ids = expanded
            .iter()
            .map(|candidate| candidate.idx)
            .collect::<Vec<_>>();

        assert!(ids.contains(&0));
        assert!(ids.contains(&2));
        assert!(!ids.contains(&1));
    }

    #[test]
    fn extend_pool_empty_seeds_yields_empty() {
        let graph = graph(&[(1, &[1.0, 0.0], 0)]);

        assert!(extend_candidate_pool(&graph, &[], 0, &[0.0, 0.0], &params(), 8).is_empty());
    }

    #[test]
    fn extend_pool_no_neighbors_passes_through_unchanged() {
        let graph = graph(&[(1, &[1.0, 0.0], 0)]);
        let seeds = vec![Candidate::admissible(0, -1.0)];

        assert_eq!(
            extend_candidate_pool(&graph, &seeds, 0, &[0.0, 0.0], &params(), 8)
                .iter()
                .map(|candidate| candidate.idx)
                .collect::<Vec<_>>(),
            vec![0]
        );
    }

    #[test]
    fn select_with_empty_pool_returns_empty() {
        let graph = graph(&[(1, &[1.0, 0.0], 0)]);

        assert!(
            select_neighbors_heuristic(&graph, Vec::new(), 4, &params(), opts(true)).is_empty()
        );
    }

    #[test]
    fn select_with_pool_smaller_than_m_returns_all() {
        let graph = graph(&[(1, &[1.0, 0.0], 0), (2, &[4.0, 0.0], 0)]);
        let selected = select_neighbors_heuristic(
            &graph,
            candidates(&graph, &[0.0, 0.0]),
            8,
            &params(),
            opts(true),
        );

        assert_eq!(selected, vec![0, 1]);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn diversity_postcondition_holds_under_random_pools(
            raw_vectors in proptest::collection::vec((-100_i16..100, -100_i16..100), 2..64),
            m in 1_usize..16,
        ) {
            let rows = raw_vectors
                .iter()
                .enumerate()
                .map(|(idx, (x, y))| ((idx + 1) as u64, vec![*x as f32, *y as f32], 0_u8))
                .collect::<Vec<_>>();
            let borrowed = rows
                .iter()
                .map(|(raw, vector, layer)| (*raw, vector.as_slice(), *layer))
                .collect::<Vec<_>>();
            let graph = graph(&borrowed);
            let params = params();
            let selected = select_neighbors_heuristic(
                &graph,
                candidates(&graph, &[0.0, 0.0]),
                m,
                &params,
                opts(false),
            );

            for (position, cand_idx) in selected.iter().copied().enumerate() {
                let cand = graph.node_by_idx(cand_idx).expect("selected candidate exists");
                let cand_score = score(&[0.0, 0.0], &cand.vector, &params);
                for selected_idx in selected.iter().copied().take(position) {
                    let existing = graph
                        .node_by_idx(selected_idx)
                        .expect("selected neighbor exists");
                    prop_assert!(
                        score(&cand.vector, &existing.vector, &params) <= cand_score + 1e-5
                    );
                }
            }
        }
    }

    fn graph(rows: &[(u64, &[f32], u8)]) -> HnswGraph {
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

    fn candidates(graph: &HnswGraph, target: &[f32]) -> Vec<Candidate> {
        let params = params();
        graph
            .iter_nodes()
            .enumerate()
            .map(|(idx, node)| {
                Candidate::admissible(idx as InternalIndex, score(target, &node.vector, &params))
            })
            .collect()
    }

    fn opts(keep_pruned_connections: bool) -> NeighborSelectionOpts {
        NeighborSelectionOpts {
            extend_candidates: false,
            keep_pruned_connections,
        }
    }

    fn params() -> HnswParams {
        HnswParams::from_config(
            &HnswConfig::with_params(2, 4, 16, 8, DistanceMetric::L2)
                .expect("test config is valid"),
        )
    }
}
