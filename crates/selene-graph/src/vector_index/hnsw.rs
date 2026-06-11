//! In-tree HNSW graph for native vector indexes.
//!
//! This module deliberately keeps HNSW as derived in-memory state. The durable
//! contract is still the vector-index registration plus primary node values;
//! rebuild recreates the graph from those values. Deletes mark entries stale so
//! search can continue traversing historical links while excluding obsolete row
//! versions from results.

#[path = "hnsw/candidate.rs"]
mod candidate;
#[path = "hnsw/links.rs"]
mod links;
#[path = "hnsw/scratch.rs"]
mod scratch;

use std::mem::size_of;

use rustc_hash::FxHashMap;
use selene_core::{
    CoreResult, HnswIndexConfig, VectorMetric, VectorMetricQuery, VectorValue, vector_squared_norm,
};

use candidate::{Candidate, MaxCandidate, MinCandidate, closer, compare_candidate};
use links::{HnswUpperLinkLayers, LevelZeroLinks};
pub(crate) use scratch::HnswSearchScratch;

const MAX_LEVEL: usize = 16;
const LEVEL_BRANCHING_BITS: u32 = 4;

/// One approximate vector-search hit over a graph row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HnswVectorHit {
    pub(crate) row: u32,
    pub(crate) distance: f64,
}

/// Estimated HNSW resident memory and structural counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct HnswMemoryUsage {
    /// Total HNSW entries, including stale deleted row versions.
    pub(crate) entries: usize,
    /// Live entries currently reachable from row membership.
    pub(crate) live_entries: usize,
    /// Stale entries retained to keep historical links traversable.
    pub(crate) deleted_entries: usize,
    /// Stored directed links across every layer.
    pub(crate) link_count: usize,
    /// Stored directed links in the level-0 layer.
    pub(crate) level_zero_link_count: usize,
    /// Stored directed links above the level-0 layer.
    pub(crate) upper_layer_link_count: usize,
    /// Maximum layer count attached to any HNSW node.
    pub(crate) max_layer_count: usize,
    /// Maximum directed links stored in a single layer for any HNSW node.
    pub(crate) max_links_per_layer: usize,
    /// Average directed links per HNSW entry, scaled by 10,000.
    pub(crate) average_links_per_entry_basis_points: usize,
    /// Estimated heap bytes owned by HNSW structures, excluding vector components.
    pub(crate) estimated_heap_bytes: usize,
    /// Component bytes reachable through HNSW vector handles.
    pub(crate) referenced_vector_bytes: usize,
}

/// Derived HNSW graph for one vector-index registration.
#[derive(Clone, Debug)]
pub(crate) struct HnswVectorIndex {
    metric: VectorMetric,
    nodes: Vec<HnswNode>,
    entry_squared_norms: Vec<f64>,
    level_zero_links: LevelZeroLinks,
    row_to_entry: FxHashMap<u32, u32>,
    entry_point: Option<u32>,
    max_level: usize,
    m: usize,
    ef_construction: usize,
}

impl HnswVectorIndex {
    /// Construct an empty HNSW graph with explicit construction parameters.
    pub(crate) fn with_config(metric: VectorMetric, config: HnswIndexConfig) -> Self {
        Self {
            metric,
            nodes: Vec::new(),
            entry_squared_norms: Vec::new(),
            level_zero_links: LevelZeroLinks::new(),
            row_to_entry: FxHashMap::default(),
            entry_point: None,
            max_level: 0,
            m: usize::from(config.max_neighbors),
            ef_construction: usize::from(config.ef_construction),
        }
    }

    /// Return the number of currently live row versions.
    #[cfg(test)]
    pub(crate) fn live_len(&self) -> usize {
        self.row_to_entry.len()
    }

    /// Insert or replace the current vector for a graph row.
    #[cfg(test)]
    pub(crate) fn insert(&mut self, row: u32, vector: VectorValue) -> CoreResult<()> {
        let mut scratch = HnswSearchScratch::default();
        self.insert_with_scratch(row, vector, &mut scratch)
    }

    /// Insert or replace the current vector while reusing caller-owned buffers.
    pub(crate) fn insert_with_scratch(
        &mut self,
        row: u32,
        vector: VectorValue,
        scratch: &mut HnswSearchScratch,
    ) -> CoreResult<()> {
        self.remove(row);
        let query_vector = vector.clone();
        let use_cached_norms = self.metric == VectorMetric::Cosine;
        let query_squared_norm = use_cached_norms.then(|| vector_squared_norm(&query_vector));
        let insert_scorer = if let Some(query_squared_norm) = query_squared_norm {
            self.metric
                .bind_query_with_squared_norm(&query_vector, query_squared_norm)?
        } else {
            self.metric.bind_query(&query_vector)?
        };

        let new_id = u32::try_from(self.nodes.len()).expect("node rows cap HNSW entries at u32");
        let new_level = level_for(row, new_id);
        let old_entry_point = self.entry_point;
        let old_max_level = self.max_level;
        self.level_zero_links.push_empty();
        self.nodes.push(HnswNode {
            row,
            vector,
            deleted: false,
            upper_links: empty_upper_link_layers(new_level),
        });
        self.record_entry_squared_norm(new_id as usize, query_squared_norm);

        let Some(mut nearest) = old_entry_point else {
            self.entry_point = Some(new_id);
            self.max_level = new_level;
            self.row_to_entry.insert(row, new_id);
            return Ok(());
        };

        let mut nearest_distance = if use_cached_norms {
            self.distance_query_to_entry_with_cached_norms(insert_scorer, nearest)?
        } else {
            self.distance_query_to_entry(insert_scorer, nearest)?
        };
        for layer in ((new_level + 1)..=old_max_level).rev() {
            (nearest, nearest_distance) = if use_cached_norms {
                self.greedy_layer_from_query_with_cached_norms(
                    insert_scorer,
                    nearest,
                    nearest_distance,
                    layer,
                )?
            } else {
                self.greedy_layer_from_query(insert_scorer, nearest, nearest_distance, layer)?
            };
        }

        let link_top = new_level.min(old_max_level);
        for layer in (0..=link_top).rev() {
            if use_cached_norms {
                self.search_layer_from_query_with_cached_norms_into(
                    insert_scorer,
                    nearest,
                    self.ef_construction,
                    layer,
                    scratch,
                )?;
            } else {
                self.search_layer_from_query_into(
                    insert_scorer,
                    nearest,
                    self.ef_construction,
                    layer,
                    scratch,
                )?;
            }
            let selected = self.select_neighbors(
                new_id,
                &scratch.result,
                self.max_links(layer),
                &mut scratch.fallback,
            )?;
            self.set_links(new_id, layer, selected.clone());
            for neighbor in &selected {
                self.add_backlink(*neighbor, new_id, layer, scratch)?;
            }
            if let Some(first) = selected.first() {
                nearest = *first;
            }
        }

        if new_level > old_max_level {
            self.entry_point = Some(new_id);
            self.max_level = new_level;
        }
        self.row_to_entry.insert(row, new_id);
        Ok(())
    }

    /// Mark the current vector for `row` stale, if present.
    pub(crate) fn remove(&mut self, row: u32) {
        let Some(entry) = self.row_to_entry.remove(&row) else {
            return;
        };
        if let Some(node) = self.nodes.get_mut(entry as usize) {
            node.deleted = true;
        }
    }

    /// Approximate top-k search over current row versions.
    pub(crate) fn search(
        &self,
        query: &VectorValue,
        k: usize,
        ef_search: usize,
    ) -> CoreResult<Vec<HnswVectorHit>> {
        let mut scratch = HnswSearchScratch::default();
        self.search_with_scratch(query, k, ef_search, &mut scratch)
    }

    /// Approximate top-k search while reusing caller-owned buffers.
    pub(crate) fn search_with_scratch(
        &self,
        query: &VectorValue,
        k: usize,
        ef_search: usize,
        scratch: &mut HnswSearchScratch,
    ) -> CoreResult<Vec<HnswVectorHit>> {
        let use_cached_norms = self.metric == VectorMetric::Cosine;
        let scorer = if use_cached_norms {
            self.metric
                .bind_query_with_squared_norm(query, vector_squared_norm(query))?
        } else {
            self.metric.bind_query(query)?
        };
        if k == 0 || self.row_to_entry.is_empty() {
            return Ok(Vec::new());
        }
        let Some(mut nearest) = self.entry_point else {
            return Ok(Vec::new());
        };
        let mut nearest_distance = if use_cached_norms {
            self.distance_query_to_entry_with_cached_norms(scorer, nearest)?
        } else {
            self.distance_query_to_entry(scorer, nearest)?
        };
        for layer in (1..=self.max_level).rev() {
            (nearest, nearest_distance) = if use_cached_norms {
                self.greedy_layer_from_query_with_cached_norms(
                    scorer,
                    nearest,
                    nearest_distance,
                    layer,
                )?
            } else {
                self.greedy_layer_from_query(scorer, nearest, nearest_distance, layer)?
            };
        }

        let ef = ef_search.max(k).max(1);
        if use_cached_norms {
            self.search_layer_from_query_with_cached_norms_into(scorer, nearest, ef, 0, scratch)?;
        } else {
            self.search_layer_from_query_into(scorer, nearest, ef, 0, scratch)?;
        }
        let mut hits = Vec::new();
        for candidate in &scratch.result {
            let node = &self.nodes[candidate.id as usize];
            if node.deleted || self.row_to_entry.get(&node.row) != Some(&candidate.id) {
                continue;
            }
            hits.push(HnswVectorHit {
                row: node.row,
                distance: candidate.distance,
            });
            if hits.len() == k {
                break;
            }
        }
        Ok(hits)
    }

    /// Compact bulk-built level-0 links after construction or rebuild.
    pub(crate) fn finish_bulk_load(&mut self) {
        self.level_zero_links.compact();
    }

    /// Return estimated HNSW memory usage.
    pub(crate) fn memory_usage(&self) -> HnswMemoryUsage {
        let entries = self.nodes.len();
        let live_entries = self.row_to_entry.len();
        let deleted_entries = self.nodes.iter().filter(|node| node.deleted).count();
        let mut link_count = 0usize;
        let mut level_zero_link_count = 0usize;
        let mut upper_layer_link_count = 0usize;
        let mut max_layer_count = 0usize;
        let mut max_links_per_layer = 0usize;
        let mut upper_link_capacity = 0usize;
        let mut layer_vec_capacity = 0usize;
        let mut referenced_vector_bytes = 0usize;
        self.level_zero_links.for_each(|layer| {
            let layer_links = layer.len();
            link_count = link_count.saturating_add(layer_links);
            level_zero_link_count = level_zero_link_count.saturating_add(layer_links);
            max_links_per_layer = max_links_per_layer.max(layer_links);
        });
        for node in &self.nodes {
            referenced_vector_bytes = referenced_vector_bytes
                .saturating_add(node.vector.dimension().saturating_mul(size_of::<f32>()));
            max_layer_count = max_layer_count.max(1 + node.upper_links.len());
            layer_vec_capacity = layer_vec_capacity.saturating_add(node.upper_links.capacity());
            for layer in &node.upper_links {
                let layer_links = layer.len();
                link_count = link_count.saturating_add(layer_links);
                max_links_per_layer = max_links_per_layer.max(layer_links);
                upper_layer_link_count = upper_layer_link_count.saturating_add(layer_links);
                upper_link_capacity = upper_link_capacity.saturating_add(layer.capacity());
            }
        }
        let average_links_per_entry_basis_points = link_count
            .saturating_mul(10_000)
            .checked_div(entries)
            .unwrap_or(0);
        let estimated_heap_bytes = self
            .nodes
            .capacity()
            .saturating_mul(size_of::<HnswNode>())
            .saturating_add(
                self.entry_squared_norms
                    .capacity()
                    .saturating_mul(size_of::<f64>()),
            )
            .saturating_add(self.level_zero_links.estimated_heap_bytes())
            .saturating_add(layer_vec_capacity.saturating_mul(size_of::<Vec<u32>>()))
            .saturating_add(upper_link_capacity.saturating_mul(size_of::<u32>()))
            .saturating_add(
                self.row_to_entry
                    .capacity()
                    .saturating_mul(size_of::<(u32, u32)>()),
            );
        HnswMemoryUsage {
            entries,
            live_entries,
            deleted_entries,
            link_count,
            level_zero_link_count,
            upper_layer_link_count,
            max_layer_count,
            max_links_per_layer,
            average_links_per_entry_basis_points,
            estimated_heap_bytes,
            referenced_vector_bytes,
        }
    }

    fn record_entry_squared_norm(&mut self, entry_id: usize, squared_norm: Option<f64>) {
        let Some(squared_norm) = squared_norm else {
            self.entry_squared_norms.clear();
            return;
        };
        if self.entry_squared_norms.len() == entry_id {
            self.entry_squared_norms.push(squared_norm);
        } else if let Some(cached) = self.entry_squared_norms.get_mut(entry_id) {
            *cached = squared_norm;
        } else {
            self.entry_squared_norms.resize(entry_id, 0.0);
            self.entry_squared_norms.push(squared_norm);
        }
    }

    fn greedy_layer_from_query(
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

    fn greedy_layer_from_query_with_cached_norms(
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

    fn search_layer_from_query_into(
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

    fn search_layer_from_query_with_cached_norms_into(
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
    fn search_layer<F>(
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

    fn select_neighbors(
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

    fn add_backlink(
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

    fn max_links(&self, layer: usize) -> usize {
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

    fn set_links(&mut self, node_id: u32, layer: usize, links: Vec<u32>) {
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

    fn distance_query_to_entry(
        &self,
        scorer: VectorMetricQuery<'_>,
        entry: u32,
    ) -> CoreResult<f64> {
        let node = &self.nodes[entry as usize];
        scorer.distance(&node.vector)
    }

    fn distance_query_to_entry_with_cached_norms(
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

#[derive(Clone, Debug)]
struct HnswNode {
    row: u32,
    vector: VectorValue,
    deleted: bool,
    upper_links: HnswUpperLinkLayers,
}

fn level_for(row: u32, ordinal: u32) -> usize {
    let mut bits = splitmix64(((u64::from(row)) << 32) ^ u64::from(ordinal));
    let mut level = 0usize;
    let mask = (1_u64 << LEVEL_BRANCHING_BITS) - 1;
    while level < MAX_LEVEL && (bits & mask) == 0 {
        level += 1;
        bits >>= LEVEL_BRANCHING_BITS;
    }
    level
}

fn empty_upper_link_layers(level: usize) -> HnswUpperLinkLayers {
    let mut links = HnswUpperLinkLayers::with_capacity(level);
    links.resize_with(level, Vec::new);
    links
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

#[cfg(test)]
#[path = "hnsw/tests.rs"]
mod tests;
