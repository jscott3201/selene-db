//! Batched IVF search helpers.

use rayon::prelude::*;
use selene_core::{CoreResult, VectorValue};

use super::{IvfVectorHit, IvfVectorIndex};

#[cfg(not(test))]
const PARALLEL_SEARCH_BATCH_MIN_QUERIES: usize = 4;
#[cfg(test)]
const PARALLEL_SEARCH_BATCH_MIN_QUERIES: usize = 2;
#[cfg(not(test))]
const PARALLEL_SEARCH_BATCH_MIN_WORK: usize = 1_024;
#[cfg(test)]
const PARALLEL_SEARCH_BATCH_MIN_WORK: usize = 8;

impl IvfVectorIndex {
    /// Approximate top-k search for several independent queries.
    pub(crate) fn search_batch(
        &self,
        queries: &[VectorValue],
        k: usize,
        search_width: usize,
    ) -> CoreResult<Vec<Vec<IvfVectorHit>>> {
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        if k == 0 || self.row_to_entry.is_empty() {
            return Ok(vec![Vec::new(); queries.len()]);
        }
        if self.should_parallelize_search_batch(queries.len(), search_width) {
            return queries
                .par_iter()
                .map(|query| self.search(query, k, search_width))
                .collect();
        }
        queries
            .iter()
            .map(|query| self.search(query, k, search_width))
            .collect()
    }

    fn should_parallelize_search_batch(&self, query_count: usize, search_width: usize) -> bool {
        query_count >= PARALLEL_SEARCH_BATCH_MIN_QUERIES
            && query_count.saturating_mul(self.search_work_estimate(search_width))
                >= PARALLEL_SEARCH_BATCH_MIN_WORK
    }

    fn search_work_estimate(&self, search_width: usize) -> usize {
        if self.centroids.is_empty() || self.lists.is_empty() {
            return self.row_to_entry.len();
        }
        self.centroids
            .len()
            .saturating_add(self.average_probed_entries(search_width))
    }

    fn average_probed_entries(&self, search_width: usize) -> usize {
        let probe_count = search_width.max(1).min(self.lists.len());
        self.assigned_entry_count
            .saturating_mul(probe_count)
            .saturating_add(self.lists.len().saturating_sub(1))
            / self.lists.len().max(1)
    }
}

#[cfg(test)]
mod tests {
    use selene_core::VectorMetric;

    use super::*;

    fn vector(values: &[f32]) -> VectorValue {
        VectorValue::new(values.to_vec()).expect("test vector is valid")
    }

    #[test]
    fn ivf_search_batch_matches_single_queries() {
        let mut index = IvfVectorIndex::new(VectorMetric::SquaredEuclidean);
        for row in 0..16 {
            index.insert(row, vector(&[row as f32, 0.0])).unwrap();
        }
        index.finish_bulk_load().unwrap();

        let queries = [vector(&[1.1, 0.0]), vector(&[9.2, 0.0])];
        assert!(index.should_parallelize_search_batch(queries.len(), index.lists.len()));

        let batch = index.search_batch(&queries, 3, index.lists.len()).unwrap();
        let singles = queries
            .iter()
            .map(|query| index.search(query, 3, index.lists.len()).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(batch, singles);
    }
}
