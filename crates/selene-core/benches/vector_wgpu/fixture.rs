use selene_core::VectorTopK;

use crate::vector_wgpu_support::{Case, TOP_K};

pub(crate) struct Fixture {
    pub(crate) case: Case,
    pub(crate) queries: Vec<f32>,
    pub(crate) candidates: Vec<f32>,
    pub(crate) norms: Vec<f32>,
    pub(crate) cpu_scores: Vec<f32>,
}

impl Fixture {
    pub(crate) fn build(case: Case) -> Self {
        let queries = flatten_seeded(case.queries, case.dimension, 0);
        let candidates = flatten_seeded(case.candidates, case.dimension, 1_000);
        let query_norms = norms(&queries, case.dimension);
        let candidate_norms = norms(&candidates, case.dimension);
        let cpu_scores = cpu_scores(case, &queries, &candidates, &query_norms, &candidate_norms);
        let norms = query_norms.into_iter().chain(candidate_norms).collect();
        Self {
            case,
            queries,
            candidates,
            norms,
            cpu_scores,
        }
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

fn cpu_scores(
    case: Case,
    queries: &[f32],
    candidates: &[f32],
    query_norms: &[f32],
    candidate_norms: &[f32],
) -> Vec<f32> {
    let mut scores = Vec::with_capacity(case.score_count());
    for (query_idx, &query_norm) in query_norms.iter().enumerate().take(case.queries) {
        let query = window(queries, query_idx, case.dimension);
        for (candidate_idx, &candidate_norm) in
            candidate_norms.iter().enumerate().take(case.candidates)
        {
            let candidate = window(candidates, candidate_idx, case.dimension);
            let dot: f32 = query
                .iter()
                .zip(candidate)
                .map(|(lhs, rhs)| lhs * rhs)
                .sum();
            let denom = query_norm.sqrt() * candidate_norm.sqrt();
            scores.push(1.0 - (dot / denom).clamp(-1.0, 1.0));
        }
    }
    scores
}

fn window(slab: &[f32], index: usize, dimension: usize) -> &[f32] {
    let start = index * dimension;
    &slab[start..start + dimension]
}
