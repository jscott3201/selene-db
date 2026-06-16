use roaring::RoaringBitmap;
use selene_core::{CoreResult, VectorMetric, VectorTopK, VectorValue};

use super::{IvfVectorIndex, support::vector_hits};

impl IvfVectorIndex {
    pub(crate) fn search_in_rows(
        &self,
        query: &VectorValue,
        k: usize,
        search_width: usize,
        allowed_rows: &RoaringBitmap,
    ) -> CoreResult<Vec<super::IvfVectorHit>> {
        if k == 0 || self.row_to_entry.is_empty() || allowed_rows.is_empty() {
            return Ok(Vec::new());
        }
        let scorer = self.metric.bind_query(query)?;
        let mut top_k = VectorTopK::new(k);
        if self.centroids.is_empty() || self.lists.is_empty() {
            let has_stale_entries = self.has_stale_entries();
            if self.metric == VectorMetric::Cosine {
                for (entry_id, entry) in self.entries.iter().enumerate() {
                    let entry_id = u32::try_from(entry_id).expect("IVF entry id fits u32");
                    if !self.is_current_entry(entry_id, entry, has_stale_entries)
                        || !allowed_rows.contains(entry.row)
                    {
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
                    if !self.is_current_entry(entry_id, entry, has_stale_entries)
                        || !allowed_rows.contains(entry.row)
                    {
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
                    if !self.is_current_entry(entry_id, entry, has_stale_entries)
                        || !allowed_rows.contains(entry.row)
                    {
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
                    if !self.is_current_entry(entry_id, entry, has_stale_entries)
                        || !allowed_rows.contains(entry.row)
                    {
                        continue;
                    }
                    let distance = scorer.distance(&entry.vector)?;
                    top_k.push_distance(entry.row, distance);
                }
            }
        }
        Ok(vector_hits(top_k))
    }
}
