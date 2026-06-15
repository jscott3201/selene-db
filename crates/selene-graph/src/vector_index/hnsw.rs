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
#[path = "hnsw/search.rs"]
mod search;

use std::mem::size_of;

use roaring::RoaringBitmap;
use rustc_hash::FxHashMap;
use selene_core::{CoreResult, HnswIndexConfig, VectorMetric, VectorValue, vector_squared_norm};

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
        self.search_with_optional_rows(query, k, ef_search, None, scratch)
    }

    /// Approximate top-k search while admitting only rows in `allowed_rows`.
    pub(crate) fn search_in_rows_with_scratch(
        &self,
        query: &VectorValue,
        k: usize,
        ef_search: usize,
        allowed_rows: &RoaringBitmap,
        scratch: &mut HnswSearchScratch,
    ) -> CoreResult<Vec<HnswVectorHit>> {
        if allowed_rows.is_empty() {
            return Ok(Vec::new());
        }
        self.search_with_optional_rows(query, k, ef_search, Some(allowed_rows), scratch)
    }

    fn search_with_optional_rows(
        &self,
        query: &VectorValue,
        k: usize,
        ef_search: usize,
        allowed_rows: Option<&RoaringBitmap>,
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
            if allowed_rows.is_some_and(|rows| !rows.contains(node.row)) {
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
