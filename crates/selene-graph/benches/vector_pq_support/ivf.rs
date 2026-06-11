use std::mem::size_of;

use selene_core::{VectorTopK, VectorValue};

use super::turbo_quant::{TurboQuantIndex, TurboQuantVariant};
use super::{CorpusProfile, DIMENSION, K, PqCorpus, cluster_count, memory_suffix, squared_l2};

#[derive(Clone, Copy, Debug)]
pub(crate) struct IvfTurboQuantVariant {
    pub(crate) name: &'static str,
    pub(crate) turbo: TurboQuantVariant,
    pub(crate) probes: usize,
}

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

#[derive(Debug)]
pub(crate) struct IvfTurboQuantFixture {
    variant: IvfTurboQuantVariant,
    corpus: PqCorpus,
    turbo: TurboQuantIndex,
    coarse: CoarsePartition,
}

impl IvfTurboQuantFixture {
    pub(crate) fn build(scale: usize, variant: IvfTurboQuantVariant) -> Self {
        Self::build_with_profile(scale, variant, CorpusProfile::Clustered)
    }

    pub(crate) fn build_with_profile(
        scale: usize,
        variant: IvfTurboQuantVariant,
        profile: CorpusProfile,
    ) -> Self {
        let corpus = PqCorpus::build_profile_cosine(scale, profile);
        let turbo = TurboQuantIndex::build(&corpus.vectors, variant.turbo);
        let coarse = CoarsePartition::build(&corpus);
        Self {
            variant,
            corpus,
            turbo,
            coarse,
        }
    }

    pub(crate) fn total_overlap(&self) -> usize {
        let mut rows = Vec::new();
        self.corpus.total_overlap(|query| {
            self.coarse
                .candidate_rows(query, self.variant.probes, &mut rows);
            self.turbo
                .search_rows(&self.corpus.vectors, query, rows.iter().copied(), K)
        })
    }

    pub(crate) fn recall_basis_points(&self) -> usize {
        self.corpus.recall_basis_points(self.total_overlap())
    }

    pub(crate) fn searched_rows(&self) -> usize {
        let mut rows = Vec::new();
        self.corpus
            .queries
            .iter()
            .map(|query| {
                self.coarse
                    .candidate_rows(query, self.variant.probes, &mut rows);
                rows.len()
            })
            .sum()
    }

    pub(crate) fn memory_suffix(&self) -> String {
        memory_suffix(
            self.turbo
                .estimated_bytes()
                .saturating_add(self.coarse.estimated_bytes()),
            self.corpus.full_vector_bytes(),
        )
    }
}
