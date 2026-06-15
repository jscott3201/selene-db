//! In-memory inverted-file vector index for native vector indexes.
//!
//! IVF stays derived from primary graph values: durable state is the vector
//! index registration plus node properties, and rebuild recreates centroids and
//! inverted lists. Search probes nearest centroids, exact-reranks candidates in
//! those lists, and skips stale row versions left by updates/deletes.

use std::mem::size_of;

use rayon::prelude::*;
use rustc_hash::FxHashMap;
use selene_core::{
    CoreResult, IvfIndexConfig, VectorMetric, VectorMetricQuery, VectorTopK, VectorValue,
    vector_squared_norm,
};

use super::config::MAX_IVF_TARGET_CENTROIDS;

#[path = "ivf/batch.rs"]
mod batch;
#[path = "ivf/filter.rs"]
mod filter;
#[path = "ivf/support.rs"]
mod support;

use support::{
    IvfEntry, average_list_len_basis_points, remove_entry_id, target_centroid_count,
    training_entry_ids, vector_hits,
};

const MAX_CENTROIDS: usize = MAX_IVF_TARGET_CENTROIDS as usize;
// Training is sampled above this point, but final list assignment is exhaustive.
const TRAINING_SAMPLE_MAX_ENTRIES: usize = MAX_CENTROIDS * 128;
const TRAINING_ITERATIONS: usize = 2;
const UNASSIGNED_LIST_ID: u32 = u32::MAX;

#[cfg(not(test))]
const PARALLEL_ASSIGNMENT_MIN_ENTRIES: usize = 4_096;
#[cfg(test)]
const PARALLEL_ASSIGNMENT_MIN_ENTRIES: usize = 8;

/// One approximate vector-search hit over a graph row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct IvfVectorHit {
    pub(crate) row: u32,
    pub(crate) distance: f64,
}

/// Estimated IVF resident memory and structural counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct IvfMemoryUsage {
    /// Total IVF entries, including stale deleted row versions.
    pub(crate) entries: usize,
    /// Live entries currently reachable from row membership.
    pub(crate) live_entries: usize,
    /// Stale entries retained until the derived index is rebuilt.
    pub(crate) deleted_entries: usize,
    /// Number of trained centroids.
    pub(crate) centroids: usize,
    /// Number of inverted lists.
    pub(crate) list_count: usize,
    /// Number of inverted lists with at least one assigned live entry.
    pub(crate) non_empty_list_count: usize,
    /// Maximum assigned live entries in one inverted list.
    pub(crate) max_list_len: usize,
    /// Average assigned live entries per inverted list, scaled by 10,000.
    pub(crate) average_list_len_basis_points: usize,
    /// Non-stale entry ids assigned to inverted lists.
    pub(crate) assigned_entries: usize,
    /// Live entries whose current vector was not part of the last centroid training pass.
    pub(crate) pending_retrain_entries: usize,
    /// Estimated heap bytes owned by IVF structures, excluding vector components.
    pub(crate) estimated_heap_bytes: usize,
    /// Component bytes reachable through IVF entry and centroid vector handles.
    pub(crate) referenced_vector_bytes: usize,
}

/// Derived IVF index for one vector-index registration.
#[derive(Clone, Debug)]
pub(crate) struct IvfVectorIndex {
    metric: VectorMetric,
    config: Option<IvfIndexConfig>,
    entries: Vec<IvfEntry>,
    entry_squared_norms: Vec<f64>,
    entry_list_ids: Vec<u32>,
    row_to_entry: FxHashMap<u32, u32>,
    centroids: Vec<VectorValue>,
    centroid_squared_norms: Vec<f64>,
    lists: Vec<Vec<u32>>,
    assigned_entry_count: usize,
    pending_retrain_entry_count: usize,
}

impl IvfVectorIndex {
    /// Construct an empty IVF index for `metric`.
    #[cfg(test)]
    pub(crate) fn new(metric: VectorMetric) -> Self {
        Self::with_config(metric, None)
    }

    /// Construct an empty IVF index for `metric` and optional construction config.
    pub(crate) fn with_config(metric: VectorMetric, config: Option<IvfIndexConfig>) -> Self {
        Self {
            metric,
            config,
            entries: Vec::new(),
            entry_squared_norms: Vec::new(),
            entry_list_ids: Vec::new(),
            row_to_entry: FxHashMap::default(),
            centroids: Vec::new(),
            centroid_squared_norms: Vec::new(),
            lists: Vec::new(),
            assigned_entry_count: 0,
            pending_retrain_entry_count: 0,
        }
    }

    /// Insert or replace the current vector for a graph row.
    pub(crate) fn insert(&mut self, row: u32, vector: VectorValue) -> CoreResult<()> {
        if let Some(entry_id) = self.row_to_entry.get(&row).copied() {
            return self.replace_entry(entry_id, vector);
        }
        let entry_id = u32::try_from(self.entries.len()).expect("node rows cap IVF entries at u32");
        let pending_retrain = self.has_trained_centroids();
        self.entries.push(IvfEntry {
            row,
            vector,
            deleted: false,
            pending_retrain,
        });
        self.entry_list_ids.push(UNASSIGNED_LIST_ID);
        self.record_entry_squared_norm(entry_id as usize);
        self.row_to_entry.insert(row, entry_id);
        self.assign_entry(entry_id)?;
        if pending_retrain {
            self.pending_retrain_entry_count += 1;
        }
        Ok(())
    }

    /// Mark the current vector for `row` stale, if present.
    pub(crate) fn remove(&mut self, row: u32) {
        let Some(entry_id) = self.row_to_entry.remove(&row) else {
            return;
        };
        if let Some(list) = self.assigned_list_for_entry(entry_id)
            && self.remove_entry_from_list(entry_id, list)
        {
            self.assigned_entry_count = self.assigned_entry_count.saturating_sub(1);
        }
        self.set_entry_list(entry_id, None);
        if let Some(node) = self.entries.get_mut(entry_id as usize) {
            if node.pending_retrain {
                self.pending_retrain_entry_count =
                    self.pending_retrain_entry_count.saturating_sub(1);
                node.pending_retrain = false;
            }
            node.deleted = true;
        }
    }

    /// Re-train centroids and rebuild inverted lists after a bulk load.
    pub(crate) fn finish_bulk_load(&mut self) -> CoreResult<()> {
        let live_entries = self.live_entry_ids();
        if live_entries.is_empty() {
            self.centroids.clear();
            self.centroid_squared_norms.clear();
            self.lists.clear();
            self.assigned_entry_count = 0;
            self.clear_entry_list_ids();
            self.mark_all_entries_trained();
            return Ok(());
        }
        let centroid_count = self.target_centroid_count(live_entries.len());
        let training_entries = training_entry_ids(&live_entries);
        self.centroids = self.seed_centroids(&training_entries, centroid_count);
        self.refine_centroids(&training_entries)?;
        self.refresh_centroid_squared_norms();
        self.rebuild_lists(&live_entries)?;
        self.mark_all_entries_trained();
        Ok(())
    }

    /// Approximate top-k search over current row versions.
    pub(crate) fn search(
        &self,
        query: &VectorValue,
        k: usize,
        search_width: usize,
    ) -> CoreResult<Vec<IvfVectorHit>> {
        if k == 0 || self.row_to_entry.is_empty() {
            return Ok(Vec::new());
        }
        let scorer = self.metric.bind_query(query)?;
        let mut top_k = VectorTopK::new(k);
        if self.centroids.is_empty() || self.lists.is_empty() {
            let has_stale_entries = self.has_stale_entries();
            if self.metric == VectorMetric::Cosine {
                for (entry_id, entry) in self.entries.iter().enumerate() {
                    let entry_id = u32::try_from(entry_id).expect("IVF entry id fits u32");
                    if !self.is_current_entry(entry_id, entry, has_stale_entries) {
                        continue;
                    }
                    let distance = scorer.distance_with_candidate_squared_norm(
                        &entry.vector,
                        self.cached_entry_squared_norm(entry_id as usize, &entry.vector),
                    )?;
                    top_k.push_distance(entry.row, distance);
                }
            } else {
                for (entry_id, entry) in self.entries.iter().enumerate() {
                    let entry_id = u32::try_from(entry_id).expect("IVF entry id fits u32");
                    if !self.is_current_entry(entry_id, entry, has_stale_entries) {
                        continue;
                    }
                    let distance = scorer.distance(&entry.vector)?;
                    top_k.push_distance(entry.row, distance);
                }
            }
            return Ok(vector_hits(top_k));
        }

        let has_stale_entries = self.has_stale_assigned_entries();
        let probe_count = search_width.max(1).min(self.centroids.len());
        let mut centroid_top_k = VectorTopK::new(probe_count);
        if self.metric == VectorMetric::Cosine {
            for (centroid_id, centroid) in self.centroids.iter().enumerate() {
                let distance = scorer.distance_with_candidate_squared_norm(
                    centroid,
                    self.cached_centroid_squared_norm(centroid_id, centroid),
                )?;
                centroid_top_k.push_distance(centroid_id, distance);
            }
        } else {
            for (centroid_id, centroid) in self.centroids.iter().enumerate() {
                let distance = scorer.distance(centroid)?;
                centroid_top_k.push_distance(centroid_id, distance);
            }
        }
        if self.metric == VectorMetric::Cosine {
            for centroid in centroid_top_k.into_hits() {
                let Some(list) = self.lists.get(centroid.key) else {
                    continue;
                };
                for &entry_id in list {
                    let entry = &self.entries[entry_id as usize];
                    if !self.is_current_entry(entry_id, entry, has_stale_entries) {
                        continue;
                    }
                    let distance = scorer.distance_with_candidate_squared_norm(
                        &entry.vector,
                        self.cached_entry_squared_norm(entry_id as usize, &entry.vector),
                    )?;
                    top_k.push_distance(entry.row, distance);
                }
            }
        } else {
            for centroid in centroid_top_k.into_hits() {
                let Some(list) = self.lists.get(centroid.key) else {
                    continue;
                };
                for &entry_id in list {
                    let entry = &self.entries[entry_id as usize];
                    if !self.is_current_entry(entry_id, entry, has_stale_entries) {
                        continue;
                    }
                    let distance = scorer.distance(&entry.vector)?;
                    top_k.push_distance(entry.row, distance);
                }
            }
        }
        Ok(vector_hits(top_k))
    }

    /// Return estimated IVF memory usage.
    pub(crate) fn memory_usage(&self) -> IvfMemoryUsage {
        let entries = self.entries.len();
        let live_entries = self.row_to_entry.len();
        let deleted_entries = self.entries.iter().filter(|entry| entry.deleted).count();
        debug_assert_eq!(
            self.assigned_entry_count,
            self.lists.iter().map(Vec::len).sum::<usize>()
        );
        let assigned_entries = self.assigned_entry_count;
        let pending_retrain_entries = self.pending_retrain_entry_count;
        let non_empty_list_count = self.lists.iter().filter(|list| !list.is_empty()).count();
        let max_list_len = self.lists.iter().map(Vec::len).max().unwrap_or_default();
        let list_capacity = self.lists.iter().map(Vec::capacity).sum::<usize>();
        let referenced_vector_bytes = self
            .entries
            .iter()
            .map(|entry| entry.vector.dimension().saturating_mul(size_of::<f32>()))
            .chain(
                self.centroids
                    .iter()
                    .map(|centroid| centroid.dimension().saturating_mul(size_of::<f32>())),
            )
            .sum();
        let estimated_heap_bytes = self
            .entries
            .capacity()
            .saturating_mul(size_of::<IvfEntry>())
            .saturating_add(
                self.entry_squared_norms
                    .capacity()
                    .saturating_mul(size_of::<f64>()),
            )
            .saturating_add(
                self.entry_list_ids
                    .capacity()
                    .saturating_mul(size_of::<u32>()),
            )
            .saturating_add(
                self.row_to_entry
                    .capacity()
                    .saturating_mul(size_of::<(u32, u32)>()),
            )
            .saturating_add(
                self.centroids
                    .capacity()
                    .saturating_mul(size_of::<VectorValue>()),
            )
            .saturating_add(
                self.centroid_squared_norms
                    .capacity()
                    .saturating_mul(size_of::<f64>()),
            )
            .saturating_add(self.lists.capacity().saturating_mul(size_of::<Vec<u32>>()))
            .saturating_add(list_capacity.saturating_mul(size_of::<u32>()));
        IvfMemoryUsage {
            entries,
            live_entries,
            deleted_entries,
            centroids: self.centroids.len(),
            list_count: self.lists.len(),
            non_empty_list_count,
            max_list_len,
            average_list_len_basis_points: average_list_len_basis_points(
                assigned_entries,
                self.lists.len(),
            ),
            assigned_entries,
            pending_retrain_entries,
            estimated_heap_bytes,
            referenced_vector_bytes,
        }
    }

    fn assign_entry(&mut self, entry_id: u32) -> CoreResult<()> {
        if let Some(list) = self.nearest_centroid_for_current_entry(entry_id)? {
            self.lists[list].push(entry_id);
            self.set_entry_list(entry_id, Some(list));
            self.assigned_entry_count += 1;
        }
        Ok(())
    }

    fn replace_entry(&mut self, entry_id: u32, vector: VectorValue) -> CoreResult<()> {
        let old_list = self.assigned_list_for_entry(entry_id);
        let new_list = self.nearest_centroid_for_vector(&vector)?;
        let pending_retrain = self.has_trained_centroids();
        let entry = &mut self.entries[entry_id as usize];
        let was_pending_retrain = entry.pending_retrain;
        entry.vector = vector;
        entry.deleted = false;
        entry.pending_retrain = entry.pending_retrain || pending_retrain;
        self.record_entry_squared_norm(entry_id as usize);
        if old_list != new_list {
            if let Some(old_list) = old_list
                && self.remove_entry_from_list(entry_id, old_list)
            {
                self.assigned_entry_count = self.assigned_entry_count.saturating_sub(1);
            }
            self.set_entry_list(entry_id, None);
            if let Some(new_list) = new_list {
                self.lists[new_list].push(entry_id);
                self.set_entry_list(entry_id, Some(new_list));
                self.assigned_entry_count += 1;
            }
        }
        if pending_retrain && !was_pending_retrain {
            self.pending_retrain_entry_count += 1;
        }
        Ok(())
    }

    fn has_trained_centroids(&self) -> bool {
        !self.centroids.is_empty() && !self.lists.is_empty()
    }

    fn target_centroid_count(&self, live_len: usize) -> usize {
        self.config
            .map(|config| usize::from(config.target_centroids).min(live_len.max(1)))
            .unwrap_or_else(|| target_centroid_count(live_len))
    }

    fn mark_all_entries_trained(&mut self) {
        for entry in &mut self.entries {
            entry.pending_retrain = false;
        }
        self.pending_retrain_entry_count = 0;
    }

    fn nearest_centroid_for_current_entry(&self, entry_id: u32) -> CoreResult<Option<usize>> {
        if self.centroids.is_empty() || self.lists.is_empty() {
            return Ok(None);
        }
        self.nearest_centroid_for_entry(entry_id).map(Some)
    }

    fn nearest_centroid_for_vector(&self, vector: &VectorValue) -> CoreResult<Option<usize>> {
        if self.centroids.is_empty() || self.lists.is_empty() {
            return Ok(None);
        }
        let scorer = if self.metric == VectorMetric::Cosine {
            self.metric
                .bind_query_with_squared_norm(vector, vector_squared_norm(vector))?
        } else {
            self.metric.bind_query(vector)?
        };
        self.nearest_centroid(scorer).map(Some)
    }

    fn remove_entry_from_list(&mut self, entry_id: u32, list_id: usize) -> bool {
        if self
            .lists
            .get_mut(list_id)
            .is_some_and(|list| remove_entry_id(list, entry_id))
        {
            return true;
        }
        for list in &mut self.lists {
            if remove_entry_id(list, entry_id) {
                return true;
            }
        }
        false
    }

    fn assigned_list_for_entry(&self, entry_id: u32) -> Option<usize> {
        self.entry_list_ids
            .get(entry_id as usize)
            .copied()
            .filter(|list| *list != UNASSIGNED_LIST_ID)
            .and_then(|list| usize::try_from(list).ok())
    }

    fn set_entry_list(&mut self, entry_id: u32, list_id: Option<usize>) {
        let value = list_id
            .map(|list| u32::try_from(list).expect("IVF list count fits u32"))
            .unwrap_or(UNASSIGNED_LIST_ID);
        if let Some(stored) = self.entry_list_ids.get_mut(entry_id as usize) {
            *stored = value;
        }
    }

    fn clear_entry_list_ids(&mut self) {
        for list_id in &mut self.entry_list_ids {
            *list_id = UNASSIGNED_LIST_ID;
        }
    }

    fn has_stale_entries(&self) -> bool {
        self.entries.len() != self.row_to_entry.len()
    }

    fn has_stale_assigned_entries(&self) -> bool {
        self.assigned_entry_count != self.row_to_entry.len()
    }

    fn is_current_entry(&self, entry_id: u32, entry: &IvfEntry, has_stale_entries: bool) -> bool {
        if !has_stale_entries {
            debug_assert!(!entry.deleted);
            return !entry.deleted;
        }
        !entry.deleted && self.row_to_entry.get(&entry.row) == Some(&entry_id)
    }

    fn record_entry_squared_norm(&mut self, entry_id: usize) {
        if self.metric != VectorMetric::Cosine {
            self.entry_squared_norms.clear();
            return;
        }
        let squared_norm = vector_squared_norm(&self.entries[entry_id].vector);
        if self.entry_squared_norms.len() == entry_id {
            self.entry_squared_norms.push(squared_norm);
        } else if let Some(cached) = self.entry_squared_norms.get_mut(entry_id) {
            *cached = squared_norm;
        } else {
            self.entry_squared_norms.resize(entry_id, 0.0);
            self.entry_squared_norms.push(squared_norm);
        }
    }

    fn live_entry_ids(&self) -> Vec<u32> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(entry_id, entry)| {
                let entry_id = u32::try_from(entry_id).expect("IVF entry id fits u32");
                (!entry.deleted && self.row_to_entry.get(&entry.row) == Some(&entry_id))
                    .then_some(entry_id)
            })
            .collect()
    }

    fn seed_centroids(&self, live_entries: &[u32], centroid_count: usize) -> Vec<VectorValue> {
        if centroid_count == 1 {
            return vec![self.entries[live_entries[0] as usize].vector.clone()];
        }
        let last = live_entries.len() - 1;
        (0..centroid_count)
            .map(|slot| {
                let source = slot.saturating_mul(last) / (centroid_count - 1);
                self.entries[live_entries[source] as usize].vector.clone()
            })
            .collect()
    }

    fn refine_centroids(&mut self, live_entries: &[u32]) -> CoreResult<()> {
        for _ in 0..TRAINING_ITERATIONS {
            if self.metric == VectorMetric::Cosine {
                self.refresh_centroid_squared_norms();
            }
            let assignments = self.assignments(live_entries)?;
            let Some(dimension) = self
                .centroids
                .first()
                .map(VectorValue::dimension)
                .filter(|dimension| *dimension > 0)
            else {
                return Ok(());
            };
            let mut sums = vec![vec![0.0f64; dimension]; self.centroids.len()];
            let mut counts = vec![0usize; self.centroids.len()];
            for (&entry_id, centroid_id) in live_entries.iter().zip(assignments) {
                counts[centroid_id] += 1;
                let vector = self.entries[entry_id as usize].vector.as_slice();
                for (sum, component) in sums[centroid_id].iter_mut().zip(vector) {
                    *sum += f64::from(*component);
                }
            }
            for (centroid_id, sum) in sums.into_iter().enumerate() {
                let count = counts[centroid_id];
                if count == 0 {
                    continue;
                }
                let inverse = 1.0 / count as f64;
                let components = sum
                    .into_iter()
                    .map(|value| (value * inverse) as f32)
                    .collect::<Vec<_>>();
                let candidate = VectorValue::new(components)?;
                if self.metric.distance(&candidate, &candidate).is_ok() {
                    self.centroids[centroid_id] = candidate;
                }
            }
        }
        Ok(())
    }

    fn refresh_centroid_squared_norms(&mut self) {
        if self.metric != VectorMetric::Cosine {
            self.centroid_squared_norms.clear();
            return;
        }
        self.centroid_squared_norms = self
            .centroids
            .iter()
            .map(vector_squared_norm)
            .collect::<Vec<_>>();
    }

    fn assignments(&self, live_entries: &[u32]) -> CoreResult<Vec<usize>> {
        if should_parallelize_assignments(live_entries.len(), self.centroids.len()) {
            return live_entries
                .par_iter()
                .map(|&entry_id| self.nearest_centroid_for_entry(entry_id))
                .collect();
        }
        live_entries
            .iter()
            .map(|&entry_id| self.nearest_centroid_for_entry(entry_id))
            .collect()
    }

    fn rebuild_lists(&mut self, live_entries: &[u32]) -> CoreResult<()> {
        let assignments = self.assignments(live_entries)?;
        let mut list_lengths = vec![0usize; self.centroids.len()];
        for &list in &assignments {
            list_lengths[list] += 1;
        }
        self.lists = list_lengths.into_iter().map(Vec::with_capacity).collect();
        self.clear_entry_list_ids();
        for (&entry_id, list) in live_entries.iter().zip(assignments) {
            self.lists[list].push(entry_id);
            self.set_entry_list(entry_id, Some(list));
        }
        self.assigned_entry_count = live_entries.len();
        Ok(())
    }

    fn nearest_centroid_for_entry(&self, entry_id: u32) -> CoreResult<usize> {
        let entry = &self.entries[entry_id as usize];
        let scorer = if self.metric == VectorMetric::Cosine {
            self.metric.bind_query_with_squared_norm(
                &entry.vector,
                self.cached_entry_squared_norm(entry_id as usize, &entry.vector),
            )?
        } else {
            self.metric.bind_query(&entry.vector)?
        };
        self.nearest_centroid(scorer)
    }

    fn nearest_centroid(&self, scorer: VectorMetricQuery<'_>) -> CoreResult<usize> {
        let mut best_id = 0usize;
        let mut best_distance = f64::INFINITY;
        if self.metric == VectorMetric::Cosine {
            for (centroid_id, centroid) in self.centroids.iter().enumerate() {
                let centroid_squared_norm =
                    self.cached_centroid_squared_norm(centroid_id, centroid);
                let distance =
                    scorer.distance_with_candidate_squared_norm(centroid, centroid_squared_norm)?;
                if distance
                    .total_cmp(&best_distance)
                    .then_with(|| centroid_id.cmp(&best_id))
                    .is_lt()
                {
                    best_id = centroid_id;
                    best_distance = distance;
                }
            }
        } else {
            for (centroid_id, centroid) in self.centroids.iter().enumerate() {
                let distance = scorer.distance(centroid)?;
                if distance
                    .total_cmp(&best_distance)
                    .then_with(|| centroid_id.cmp(&best_id))
                    .is_lt()
                {
                    best_id = centroid_id;
                    best_distance = distance;
                }
            }
        }
        Ok(best_id)
    }

    fn cached_centroid_squared_norm(&self, centroid_id: usize, centroid: &VectorValue) -> f64 {
        self.centroid_squared_norms
            .get(centroid_id)
            .copied()
            .filter(|norm| *norm != 0.0)
            .unwrap_or_else(|| vector_squared_norm(centroid))
    }

    fn cached_entry_squared_norm(&self, entry_id: usize, vector: &VectorValue) -> f64 {
        self.entry_squared_norms
            .get(entry_id)
            .copied()
            .filter(|norm| *norm != 0.0)
            .unwrap_or_else(|| vector_squared_norm(vector))
    }
}

fn should_parallelize_assignments(live_len: usize, centroid_count: usize) -> bool {
    live_len >= PARALLEL_ASSIGNMENT_MIN_ENTRIES && centroid_count > 1
}

#[cfg(test)]
#[path = "ivf/tests.rs"]
mod tests;
