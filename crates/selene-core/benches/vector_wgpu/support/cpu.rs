use rayon::prelude::*;
use selene_core::VectorTopK;

use crate::vector_wgpu_case::{HOT_SHARD_REUSE_BATCHES, TOP_K};
use crate::vector_wgpu_fixture::Fixture;

use super::WgpuBench;

impl WgpuBench {
    pub(crate) fn cpu_parallel_score_top_k(&self) -> usize {
        parallel_score_top_k(
            self.dimension,
            self.candidate_count,
            self.query_count() as usize,
            &self.queries,
            &self.candidates,
            &self.norms,
        )
    }

    pub(crate) fn cpu_parallel_score_top_k_hot_shard_reuse(&self) -> usize {
        (0..HOT_SHARD_REUSE_BATCHES)
            .map(|_| self.cpu_parallel_score_top_k())
            .sum()
    }
}

pub(crate) fn fixture_parallel_score_top_k(fixture: &Fixture) -> usize {
    parallel_score_top_k(
        fixture.case.dimension,
        fixture.case.candidates,
        fixture.case.queries,
        &fixture.queries,
        &fixture.candidates,
        &fixture.norms,
    )
}

pub(crate) fn fixture_parallel_score_top_k_hot_shard_reuse(fixture: &Fixture) -> usize {
    (0..HOT_SHARD_REUSE_BATCHES)
        .map(|_| fixture_parallel_score_top_k(fixture))
        .sum()
}

pub(super) fn cpu_top_k_count(scores: &[f32], candidate_count: usize) -> usize {
    let mut retained = 0;
    for query_scores in scores.chunks_exact(candidate_count) {
        let mut top_k = VectorTopK::new(TOP_K);
        for (candidate_idx, &distance) in query_scores.iter().enumerate() {
            top_k.push_distance(candidate_idx, f64::from(distance));
        }
        retained += top_k.into_hits().len();
    }
    retained
}

pub(super) fn cpu_merge_partial_top_k_count(
    distances: &[f32],
    indices: &[u32],
    block_count: usize,
) -> usize {
    top_k_indices_from_partials(distances, indices, block_count)
        .into_iter()
        .map(|hits| hits.len())
        .sum()
}

pub(super) fn top_k_indices_from_partials(
    distances: &[f32],
    indices: &[u32],
    block_count: usize,
) -> Vec<Vec<usize>> {
    let query_width = block_count * TOP_K;
    distances
        .chunks_exact(query_width)
        .zip(indices.chunks_exact(query_width))
        .map(|(query_distances, query_indices)| {
            let mut top_k = VectorTopK::new(TOP_K);
            for (&distance, &candidate_idx) in query_distances.iter().zip(query_indices) {
                if candidate_idx != u32::MAX {
                    top_k.push_distance(candidate_idx as usize, f64::from(distance));
                }
            }
            top_k.into_hits().into_iter().map(|hit| hit.key).collect()
        })
        .collect()
}

fn parallel_score_top_k(
    dimension: usize,
    candidate_count: usize,
    query_count: usize,
    queries: &[f32],
    candidates: &[f32],
    norms: &[f32],
) -> usize {
    (0..query_count)
        .into_par_iter()
        .map(|query_idx| {
            let query = window(queries, query_idx, dimension);
            let query_norm = norms[query_idx];
            let mut top_k = VectorTopK::new(TOP_K);
            for candidate_idx in 0..candidate_count {
                let candidate = window(candidates, candidate_idx, dimension);
                let dot: f32 = query
                    .iter()
                    .zip(candidate)
                    .map(|(lhs, rhs)| lhs * rhs)
                    .sum();
                let denom = query_norm.sqrt() * norms[query_count + candidate_idx].sqrt();
                let similarity = (dot / denom).clamp(-1.0, 1.0);
                top_k.push_distance(candidate_idx, f64::from(1.0 - similarity));
            }
            top_k.into_hits().len()
        })
        .sum()
}

fn window(slab: &[f32], index: usize, dimension: usize) -> &[f32] {
    let start = index * dimension;
    &slab[start..start + dimension]
}
