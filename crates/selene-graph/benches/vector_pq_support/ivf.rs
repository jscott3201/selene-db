use std::mem::size_of;

use selene_core::{VectorTopK, VectorValue};

use super::{DIMENSION, PqCorpus, cluster_count, squared_l2};

#[derive(Debug)]
pub(crate) struct CoarsePartition {
    centroids: Vec<f32>,
    lists: Vec<Vec<usize>>,
}

impl CoarsePartition {
    pub(crate) fn build(corpus: &PqCorpus) -> Self {
        let partitions = cluster_count(corpus.scale);
        let mut sums = vec![0.0f64; partitions * DIMENSION];
        let mut counts = vec![0usize; partitions];
        let mut lists = (0..partitions).map(|_| Vec::new()).collect::<Vec<_>>();
        for (row, vector) in corpus.vectors.iter().enumerate() {
            let partition = row % partitions;
            lists[partition].push(row);
            counts[partition] += 1;
            for dim in 0..DIMENSION {
                sums[partition * DIMENSION + dim] += f64::from(vector.as_slice()[dim]);
            }
        }

        let mut centroids = vec![0.0f32; partitions * DIMENSION];
        for partition in 0..partitions {
            let inverse = 1.0 / counts[partition] as f64;
            for dim in 0..DIMENSION {
                centroids[partition * DIMENSION + dim] =
                    (sums[partition * DIMENSION + dim] * inverse) as f32;
            }
        }
        Self { centroids, lists }
    }

    pub(crate) fn candidate_rows(&self, query: &VectorValue, probes: usize, rows: &mut Vec<usize>) {
        let mut centroid_top_k = VectorTopK::new(probes.max(1).min(self.lists.len()));
        for centroid in 0..self.lists.len() {
            centroid_top_k.push_distance(centroid, self.centroid_distance(query, centroid));
        }

        rows.clear();
        for centroid in centroid_top_k.into_hits() {
            rows.extend_from_slice(&self.lists[centroid.key]);
        }
    }

    pub(crate) fn estimated_bytes(&self) -> usize {
        let list_rows = self.lists.iter().map(Vec::len).sum::<usize>();
        self.centroids
            .len()
            .saturating_mul(size_of::<f32>())
            .saturating_add(list_rows.saturating_mul(size_of::<usize>()))
    }

    fn centroid_distance(&self, query: &VectorValue, centroid: usize) -> f64 {
        let offset = centroid * DIMENSION;
        squared_l2(
            query.as_slice(),
            &self.centroids[offset..offset + DIMENSION],
        )
    }
}
