use selene_core::VectorTopK;

use crate::vector_wgpu_case::{Case, TOP_K};

pub(crate) struct Fixture {
    pub(crate) case: Case,
    pub(crate) queries: Vec<f32>,
    pub(crate) candidates: Vec<f32>,
    pub(crate) norms: Vec<f32>,
}

impl Fixture {
    pub(crate) fn build(case: Case) -> Self {
        let queries = flatten_seeded(case.queries, case.dimension, 0);
        let candidates = flatten_seeded(case.candidates, case.dimension, 1_000);
        let query_norms = norms(&queries, case.dimension);
        let candidate_norms = norms(&candidates, case.dimension);
        let norms = query_norms.into_iter().chain(candidate_norms).collect();
        Self {
            case,
            queries,
            candidates,
            norms,
        }
    }

    pub(crate) fn cpu_score(&self, score_idx: usize) -> f32 {
        let query_idx = score_idx / self.case.candidates;
        let candidate_idx = score_idx % self.case.candidates;
        let query = window(&self.queries, query_idx, self.case.dimension);
        let candidate = window(&self.candidates, candidate_idx, self.case.dimension);
        let query_norm = self.norms[query_idx];
        let candidate_norm = self.norms[self.case.queries + candidate_idx];
        let dot: f32 = query
            .iter()
            .zip(candidate)
            .map(|(lhs, rhs)| lhs * rhs)
            .sum();
        let denom = query_norm.sqrt() * candidate_norm.sqrt();
        1.0 - (dot / denom).clamp(-1.0, 1.0)
    }
}

pub(crate) fn top_k_indices_from_scores(scores: &[f32], candidate_count: usize) -> Vec<Vec<usize>> {
    scores
        .chunks_exact(candidate_count)
        .map(|query_scores| {
            let mut top_k = VectorTopK::new(TOP_K);
            for (candidate_idx, &distance) in query_scores.iter().enumerate() {
                top_k.push_distance(candidate_idx, f64::from(distance));
            }
            top_k.into_hits().into_iter().map(|hit| hit.key).collect()
        })
        .collect()
}

fn flatten_seeded(count: usize, dimension: usize, seed_base: usize) -> Vec<f32> {
    let mut vectors = Vec::with_capacity(count * dimension);
    for vector_idx in 0..count {
        vectors.extend(vector_components_seeded(dimension, vector_idx + seed_base));
    }
    vectors
}

fn vector_components_seeded(dim: usize, seed: usize) -> impl Iterator<Item = f32> {
    (0..dim).map(move |idx| (((idx * 31 + seed * 17) % 1_021) as f32 - 510.0) / 256.0)
}

fn norms(vectors: &[f32], dimension: usize) -> Vec<f32> {
    vectors
        .chunks_exact(dimension)
        .map(|vector| vector.iter().map(|component| component * component).sum())
        .collect()
}

fn window(slab: &[f32], index: usize, dimension: usize) -> &[f32] {
    let start = index * dimension;
    &slab[start..start + dimension]
}
