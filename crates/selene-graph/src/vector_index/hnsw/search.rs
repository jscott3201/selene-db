//! Search-layer traversal and link-maintenance helpers for the in-tree HNSW index.

use selene_core::{CoreResult, VectorMetric, VectorMetricQuery, VectorValue, vector_squared_norm};

use super::candidate::{Candidate, MaxCandidate, MinCandidate, closer, compare_candidate};
use super::{HnswSearchScratch, HnswVectorIndex};

impl HnswVectorIndex {
    pub(super) fn greedy_layer_from_query(
        &self,
        scorer: VectorMetricQuery<'_>,
        entry: u32,
        entry_distance: f64,
        layer: usize,
    ) -> CoreResult<(u32, f64)> {
        self.greedy_layer(entry, entry_distance, layer, |candidate| {
            self.distance_query_to_entry(scorer, candidate)
        })
    }

    pub(super) fn greedy_layer_from_query_with_cached_norms(
        &self,
        scorer: VectorMetricQuery<'_>,
        entry: u32,
        entry_distance: f64,
        layer: usize,
    ) -> CoreResult<(u32, f64)> {
        self.greedy_layer(entry, entry_distance, layer, |candidate| {
            self.distance_query_to_entry_with_cached_norms(scorer, candidate)
        })
    }

    fn greedy_layer<F>(
        &self,
        mut nearest: u32,
        mut nearest_distance: f64,
        layer: usize,
        mut distance: F,
    ) -> CoreResult<(u32, f64)>
    where
        F: FnMut(u32) -> CoreResult<f64>,
    {
        loop {
            let mut improved = false;
            for neighbor in self.links_at(nearest, layer) {
                let neighbor_distance = distance(*neighbor)?;
                if closer(neighbor_distance, *neighbor, nearest_distance, nearest) {
                    nearest = *neighbor;
                    nearest_distance = neighbor_distance;
                    improved = true;
                }
            }
            if !improved {
                return Ok((nearest, nearest_distance));
            }
        }
    }

    pub(super) fn search_layer_from_query_into(
        &self,
        scorer: VectorMetricQuery<'_>,
        entry: u32,
        ef: usize,
        layer: usize,
        scratch: &mut HnswSearchScratch,
    ) -> CoreResult<()> {
        self.search_layer_into(entry, ef, layer, scratch, |candidate| {
            self.distance_query_to_entry(scorer, candidate)
        })
    }

    pub(super) fn search_layer_from_query_with_cached_norms_into(
        &self,
        scorer: VectorMetricQuery<'_>,
        entry: u32,
        ef: usize,
        layer: usize,
        scratch: &mut HnswSearchScratch,
    ) -> CoreResult<()> {
        self.search_layer_into(entry, ef, layer, scratch, |candidate| {
            self.distance_query_to_entry_with_cached_norms(scorer, candidate)
        })
    }

    #[cfg(test)]
    pub(super) fn search_layer<F>(
        &self,
        entry: u32,
        ef: usize,
        layer: usize,
        distance: F,
    ) -> CoreResult<Vec<Candidate>>
    where
        F: FnMut(u32) -> CoreResult<f64>,
    {
        let mut scratch = HnswSearchScratch::default();
        self.search_layer_into(entry, ef, layer, &mut scratch, distance)?;
        Ok(scratch.result.clone())
    }

    fn search_layer_into<F>(
        &self,
        entry: u32,
        ef: usize,
        layer: usize,
        scratch: &mut HnswSearchScratch,
        mut distance: F,
    ) -> CoreResult<()>
    where
        F: FnMut(u32) -> CoreResult<f64>,
    {
        let ef = ef.max(1);
        let entry_distance = distance(entry)?;
        let search_width = ef.min(self.nodes.len()).saturating_add(1);
        scratch.reset_layer(self.nodes.len(), search_width);
        let entry_was_new = scratch.visited.visit(entry);
        debug_assert!(entry_was_new);
        scratch
            .candidates
            .push(MinCandidate::new(entry, entry_distance));
        scratch.best.push(MaxCandidate::new(entry, entry_distance));

        while let Some(current) = scratch.candidates.pop() {
            let Some(worst) = scratch.best.peek() else {
                break;
            };
            if current.distance > worst.distance {
                break;
            }
            for neighbor in self.links_at(current.id, layer) {
                if !scratch.visited.visit(*neighbor) {
                    continue;
                }
                let neighbor_distance = distance(*neighbor)?;
                let admit = scratch.best.len() < ef
                    || scratch.best.peek().is_some_and(|worst| {
                        closer(neighbor_distance, *neighbor, worst.distance, worst.id)
                    });
                if admit {
                    scratch
                        .candidates
                        .push(MinCandidate::new(*neighbor, neighbor_distance));
                    scratch
                        .best
                        .push(MaxCandidate::new(*neighbor, neighbor_distance));
                    if scratch.best.len() > ef {
                        scratch.best.pop();
                    }
                }
            }
        }

        while let Some(candidate) = scratch.best.pop() {
            scratch.result.push(Candidate {
                id: candidate.id,
                distance: candidate.distance,
            });
        }
        scratch.result.sort_by(compare_candidate);
        Ok(())
    }

    pub(super) fn select_neighbors(
        &self,
        query_id: u32,
        candidates: &[Candidate],
        max_links: usize,
        fallback: &mut Vec<u32>,
    ) -> CoreResult<Vec<u32>> {
        let mut selected = Vec::with_capacity(max_links);
        fallback.clear();
        fallback.reserve(candidates.len().saturating_sub(max_links));
        for candidate in candidates {
            if candidate.id == query_id {
                continue;
            }
            if self.is_diverse_neighbor(candidate.id, candidate.distance, &selected)? {
                selected.push(candidate.id);
                if selected.len() == max_links {
                    return Ok(selected);
                }
            } else {
                fallback.push(candidate.id);
            }
        }
        for candidate in fallback.iter().copied() {
            if selected.len() == max_links {
                break;
            }
            if !selected.contains(&candidate) {
                selected.push(candidate);
            }
        }
        Ok(selected)
    }

    fn is_diverse_neighbor(
        &self,
        candidate_id: u32,
        query_distance: f64,
        selected: &[u32],
    ) -> CoreResult<bool> {
        for selected_id in selected {
            let neighbor_distance = self.distance_to_entry(candidate_id, *selected_id)?;
            if neighbor_distance < query_distance {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn add_backlink(
        &mut self,
        node_id: u32,
        neighbor: u32,
        layer: usize,
        scratch: &mut HnswSearchScratch,
    ) -> CoreResult<()> {
        let max_links = self.max_links(layer);
        {
            let links = self.links_mut(node_id, layer);
            if !links.contains(&neighbor) {
                links.push(neighbor);
            }
        }
        self.prune_links(node_id, layer, max_links, scratch)
    }

    fn prune_links(
        &mut self,
        node_id: u32,
        layer: usize,
        max_links: usize,
        scratch: &mut HnswSearchScratch,
    ) -> CoreResult<()> {
        let links = self.links_at(node_id, layer);
        scratch.reset_prune(links.len());
        for neighbor in links.iter().copied() {
            scratch.prune_candidates.push(Candidate {
                id: neighbor,
                distance: self.distance_to_entry(node_id, neighbor)?,
            });
        }
        scratch.prune_candidates.sort_by(compare_candidate);
        scratch
            .prune_candidates
            .dedup_by_key(|candidate| candidate.id);
        let selected = self.select_neighbors(
            node_id,
            &scratch.prune_candidates,
            max_links,
            &mut scratch.fallback,
        )?;
        self.set_links(node_id, layer, selected);
        Ok(())
    }

    pub(super) fn max_links(&self, layer: usize) -> usize {
        if layer == 0 { self.m * 2 } else { self.m }
    }

    fn links_at(&self, node_id: u32, layer: usize) -> &[u32] {
        if layer == 0 {
            return self.level_zero_links.get(node_id);
        }
        self.nodes
            .get(node_id as usize)
            .and_then(|node| node.upper_links.get(layer - 1))
            .map_or(&[], Vec::as_slice)
    }

    fn links_mut(&mut self, node_id: u32, layer: usize) -> &mut Vec<u32> {
        if layer == 0 {
            return self.level_zero_links.get_mut(node_id);
        }
        self.nodes[node_id as usize]
            .upper_links
            .get_mut(layer - 1)
            .expect("HNSW node has requested upper layer")
    }

    pub(super) fn set_links(&mut self, node_id: u32, layer: usize, links: Vec<u32>) {
        if layer == 0 {
            self.level_zero_links.replace(node_id, links);
        } else {
            self.nodes[node_id as usize].upper_links[layer - 1] = links;
        }
    }

    fn distance_to_entry(&self, lhs: u32, rhs: u32) -> CoreResult<f64> {
        let lhs_node = &self.nodes[lhs as usize];
        let rhs_node = &self.nodes[rhs as usize];
        if self.metric == VectorMetric::Cosine {
            let scorer = self.metric.bind_query_with_squared_norm(
                &lhs_node.vector,
                self.cached_entry_squared_norm(lhs as usize, &lhs_node.vector),
            )?;
            scorer.distance_with_candidate_squared_norm(
                &rhs_node.vector,
                self.cached_entry_squared_norm(rhs as usize, &rhs_node.vector),
            )
        } else {
            self.metric.distance(&lhs_node.vector, &rhs_node.vector)
        }
    }

    pub(super) fn distance_query_to_entry(
        &self,
        scorer: VectorMetricQuery<'_>,
        entry: u32,
    ) -> CoreResult<f64> {
        let node = &self.nodes[entry as usize];
        scorer.distance(&node.vector)
    }

    pub(super) fn distance_query_to_entry_with_cached_norms(
        &self,
        scorer: VectorMetricQuery<'_>,
        entry: u32,
    ) -> CoreResult<f64> {
        let node = &self.nodes[entry as usize];
        scorer.distance_with_candidate_squared_norm(
            &node.vector,
            self.cached_entry_squared_norm(entry as usize, &node.vector),
        )
    }

    fn cached_entry_squared_norm(&self, entry_id: usize, vector: &VectorValue) -> f64 {
        self.entry_squared_norms
            .get(entry_id)
            .copied()
            .filter(|norm| *norm != 0.0)
            .unwrap_or_else(|| vector_squared_norm(vector))
    }
}
