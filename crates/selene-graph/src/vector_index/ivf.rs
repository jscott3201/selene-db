//! In-memory inverted-file vector index for native vector indexes.
//!
//! IVF stays derived from primary graph values: durable state is the vector
//! index registration plus node properties, and rebuild recreates centroids and
//! inverted lists. Search probes nearest centroids, exact-reranks candidates in
//! those lists, and skips stale row versions left by updates/deletes.

use std::mem::size_of;

use rustc_hash::FxHashMap;
use selene_core::{CoreResult, VectorMetric, VectorTopK, VectorValue};

const MAX_CENTROIDS: usize = 256;
const TRAINING_ITERATIONS: usize = 2;

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
    /// Non-stale entry ids assigned to inverted lists.
    pub(crate) assigned_entries: usize,
    /// Estimated heap bytes owned by IVF structures, excluding vector components.
    pub(crate) estimated_heap_bytes: usize,
    /// Component bytes reachable through IVF entry and centroid vector handles.
    pub(crate) referenced_vector_bytes: usize,
}

/// Derived IVF index for one vector-index registration.
#[derive(Clone, Debug)]
pub(crate) struct IvfVectorIndex {
    metric: VectorMetric,
    entries: Vec<IvfEntry>,
    row_to_entry: FxHashMap<u32, u32>,
    centroids: Vec<VectorValue>,
    lists: Vec<Vec<u32>>,
}

impl IvfVectorIndex {
    /// Construct an empty IVF index for `metric`.
    pub(crate) fn new(metric: VectorMetric) -> Self {
        Self {
            metric,
            entries: Vec::new(),
            row_to_entry: FxHashMap::default(),
            centroids: Vec::new(),
            lists: Vec::new(),
        }
    }

    /// Insert or replace the current vector for a graph row.
    pub(crate) fn insert(&mut self, row: u32, vector: VectorValue) -> CoreResult<()> {
        self.remove(row);
        let entry_id = u32::try_from(self.entries.len()).expect("node rows cap IVF entries at u32");
        self.entries.push(IvfEntry {
            row,
            vector,
            deleted: false,
        });
        self.row_to_entry.insert(row, entry_id);
        self.assign_entry(entry_id)
    }

    /// Mark the current vector for `row` stale, if present.
    pub(crate) fn remove(&mut self, row: u32) {
        let Some(entry) = self.row_to_entry.remove(&row) else {
            return;
        };
        if let Some(node) = self.entries.get_mut(entry as usize) {
            node.deleted = true;
        }
    }

    /// Re-train centroids and rebuild inverted lists after a bulk load.
    pub(crate) fn finish_bulk_load(&mut self) -> CoreResult<()> {
        let live_entries = self.live_entry_ids();
        if live_entries.is_empty() {
            self.centroids.clear();
            self.lists.clear();
            return Ok(());
        }
        let centroid_count = target_centroid_count(live_entries.len());
        self.centroids = self.seed_centroids(&live_entries, centroid_count);
        self.refine_centroids(&live_entries)?;
        self.rebuild_lists(&live_entries)?;
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
            for entry in &self.entries {
                if entry.deleted || !self.row_to_entry.contains_key(&entry.row) {
                    continue;
                }
                let distance = scorer.distance(&entry.vector)?;
                top_k.push_distance(entry.row, distance);
            }
            return Ok(vector_hits(top_k));
        }

        let probe_count = search_width.max(1).min(self.centroids.len());
        let mut centroid_top_k = VectorTopK::new(probe_count);
        for (centroid_id, centroid) in self.centroids.iter().enumerate() {
            let distance = scorer.distance(centroid)?;
            centroid_top_k.push_distance(centroid_id, distance);
        }
        for centroid in centroid_top_k.into_hits() {
            let Some(list) = self.lists.get(centroid.key) else {
                continue;
            };
            for &entry_id in list {
                let entry = &self.entries[entry_id as usize];
                if entry.deleted || self.row_to_entry.get(&entry.row) != Some(&entry_id) {
                    continue;
                }
                let distance = scorer.distance(&entry.vector)?;
                top_k.push_distance(entry.row, distance);
            }
        }
        Ok(vector_hits(top_k))
    }

    /// Return estimated IVF memory usage.
    pub(crate) fn memory_usage(&self) -> IvfMemoryUsage {
        let entries = self.entries.len();
        let live_entries = self.row_to_entry.len();
        let deleted_entries = self.entries.iter().filter(|entry| entry.deleted).count();
        let assigned_entries = self.lists.iter().map(Vec::len).sum();
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
                self.row_to_entry
                    .capacity()
                    .saturating_mul(size_of::<(u32, u32)>()),
            )
            .saturating_add(
                self.centroids
                    .capacity()
                    .saturating_mul(size_of::<VectorValue>()),
            )
            .saturating_add(self.lists.capacity().saturating_mul(size_of::<Vec<u32>>()))
            .saturating_add(list_capacity.saturating_mul(size_of::<u32>()));
        IvfMemoryUsage {
            entries,
            live_entries,
            deleted_entries,
            centroids: self.centroids.len(),
            list_count: self.lists.len(),
            assigned_entries,
            estimated_heap_bytes,
            referenced_vector_bytes,
        }
    }

    fn assign_entry(&mut self, entry_id: u32) -> CoreResult<()> {
        if self.centroids.is_empty() || self.lists.is_empty() {
            return Ok(());
        }
        let list = self.nearest_centroid(&self.entries[entry_id as usize].vector)?;
        self.lists[list].push(entry_id);
        Ok(())
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

    fn assignments(&self, live_entries: &[u32]) -> CoreResult<Vec<usize>> {
        let mut assignments = Vec::with_capacity(live_entries.len());
        for &entry_id in live_entries {
            assignments.push(self.nearest_centroid(&self.entries[entry_id as usize].vector)?);
        }
        Ok(assignments)
    }

    fn rebuild_lists(&mut self, live_entries: &[u32]) -> CoreResult<()> {
        self.lists = vec![Vec::new(); self.centroids.len()];
        for &entry_id in live_entries {
            let list = self.nearest_centroid(&self.entries[entry_id as usize].vector)?;
            self.lists[list].push(entry_id);
        }
        Ok(())
    }

    fn nearest_centroid(&self, vector: &VectorValue) -> CoreResult<usize> {
        let scorer = self.metric.bind_query(vector)?;
        let mut best_id = 0usize;
        let mut best_distance = f64::INFINITY;
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
        Ok(best_id)
    }
}

#[derive(Clone, Debug)]
struct IvfEntry {
    row: u32,
    vector: VectorValue,
    deleted: bool,
}

fn target_centroid_count(live_len: usize) -> usize {
    ceil_sqrt(live_len).clamp(1, MAX_CENTROIDS)
}

fn ceil_sqrt(value: usize) -> usize {
    let mut root = (value as f64).sqrt() as usize;
    while root.saturating_mul(root) < value {
        root += 1;
    }
    while root > 1 && (root - 1).saturating_mul(root - 1) >= value {
        root -= 1;
    }
    root
}

fn vector_hits(top_k: VectorTopK<u32>) -> Vec<IvfVectorHit> {
    top_k
        .into_hits()
        .into_iter()
        .map(|hit| IvfVectorHit {
            row: hit.key,
            distance: hit.distance,
        })
        .collect()
}

#[cfg(test)]
#[path = "ivf/tests.rs"]
mod tests;
