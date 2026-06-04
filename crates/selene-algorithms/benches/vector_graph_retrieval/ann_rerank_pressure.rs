//! ANN search-hit candidate-set rerank rows for graph/vector retrieval.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::{CancellationChecker, NodeId, VectorMetric};
use selene_graph::VectorCandidateSet;

use crate::common::scale_label;

use super::support::{FACTS_PER_TOPIC, RESULT_K, WIDE_SEED_K, basis_points, vector_scales};
use super::{MemoryRetrievalFixture, Query, RetrievalQuality, TopologyNoise};

const ANN_RERANK_STRATEGIES: &[AnnRerankStrategy] = &[
    AnnRerankStrategy::AnnWideHitSet,
    AnnRerankStrategy::AnnWideActiveIntersection,
    AnnRerankStrategy::AnnWideDependencyUnion,
    AnnRerankStrategy::GraphDependencyCandidateSet,
];

#[derive(Clone, Copy)]
enum AnnRerankStrategy {
    AnnWideHitSet,
    AnnWideActiveIntersection,
    AnnWideDependencyUnion,
    GraphDependencyCandidateSet,
}

impl AnnRerankStrategy {
    const fn name(self) -> &'static str {
        match self {
            Self::AnnWideHitSet => "ann_wide_hit_set_batch_rerank",
            Self::AnnWideActiveIntersection => "ann_wide_active_intersection_batch_rerank",
            Self::AnnWideDependencyUnion => "ann_wide_dependency_union_batch_rerank",
            Self::GraphDependencyCandidateSet => "graph_dependency_candidate_set_batch",
        }
    }
}

pub(super) fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_ann_rerank_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build_with_topology(
            scale,
            TopologyNoise::NoisySparseMultiHopContradictedActiveHints,
        );
        for &strategy in ANN_RERANK_STRATEGIES {
            let avg_candidates = fixture.average_ann_rerank_candidates(strategy);
            let quality = fixture.ann_rerank_quality(strategy);
            group.throughput(Throughput::Elements(
                (fixture.query_count() * avg_candidates) as u64,
            ));
            group.bench_function(
                BenchmarkId::new(
                    strategy.name(),
                    format!(
                        "{}_q{}_c{}_covbp{}_curbp{}_precbp{}",
                        scale_label(fixture.scale()),
                        fixture.query_count(),
                        avg_candidates,
                        basis_points(quality.coverage, fixture.query_count() * FACTS_PER_TOPIC),
                        basis_points(
                            quality.current_coverage,
                            fixture.query_count() * FACTS_PER_TOPIC
                        ),
                        basis_points(quality.precision, fixture.query_count() * RESULT_K),
                    ),
                ),
                |b| {
                    b.iter(|| {
                        black_box(fixture.ann_rerank_total_coverage(strategy));
                    });
                },
            );
        }
    }
    group.finish();
}

impl MemoryRetrievalFixture {
    fn ann_rerank_quality(&self, strategy: AnnRerankStrategy) -> RetrievalQuality {
        let mut queries = Vec::with_capacity(self.query_count());
        let mut candidate_sets = Vec::with_capacity(self.query_count());
        let mut max_candidates = 0;
        for query in &self.queries {
            let candidates = self.ann_rerank_candidate_set(query, strategy);
            max_candidates = max_candidates.max(candidates.len());
            queries.push(query.vector.clone());
            candidate_sets.push(candidates);
        }

        let batch_hits = self
            .graph
            .score_vector_candidate_sets_batch_checked(
                &self.embedding_key,
                &queries,
                &candidate_sets,
                VectorMetric::Cosine,
                max_candidates,
                CancellationChecker::disabled(),
            )
            .expect("bench ANN rerank candidate-set scoring succeeds");

        self.queries
            .iter()
            .zip(batch_hits)
            .map(|(query, hits)| {
                let selected = self.select_from_candidates(query, hits, true, false, true);
                self.selected_quality(query, selected)
            })
            .fold(RetrievalQuality::default(), |mut total, next| {
                total.coverage += next.coverage;
                total.current_coverage += next.current_coverage;
                total.precision += next.precision;
                total
            })
    }

    fn ann_rerank_total_coverage(&self, strategy: AnnRerankStrategy) -> usize {
        self.ann_rerank_quality(strategy).coverage
    }

    fn average_ann_rerank_candidates(&self, strategy: AnnRerankStrategy) -> usize {
        self.queries
            .iter()
            .map(|query| self.ann_rerank_candidate_set(query, strategy).len())
            .sum::<usize>()
            .checked_div(self.query_count())
            .unwrap_or(0)
    }

    fn ann_rerank_candidate_set(
        &self,
        query: &Query,
        strategy: AnnRerankStrategy,
    ) -> VectorCandidateSet {
        match strategy {
            AnnRerankStrategy::AnnWideHitSet => self.ann_wide_hit_set(query),
            AnnRerankStrategy::AnnWideActiveIntersection => self
                .ann_wide_hit_set(query)
                .intersection(&self.graph_unresolved_current_candidate_set),
            AnnRerankStrategy::AnnWideDependencyUnion => self
                .ann_wide_hit_set(query)
                .union(&self.dependency_candidate_set(query)),
            AnnRerankStrategy::GraphDependencyCandidateSet => self.dependency_candidate_set(query),
        }
    }

    fn ann_wide_hit_set(&self, query: &Query) -> VectorCandidateSet {
        VectorCandidateSet::from_search_hits(self.ann_hits(query, WIDE_SEED_K))
    }

    fn dependency_candidate_set(&self, query: &Query) -> VectorCandidateSet {
        let candidates = self
            .graph
            .outgoing_edges(query.anchor)
            .map_or_else(Vec::new, |edges| {
                edges
                    .iter()
                    .filter(|edge| edge.label == self.depends_edge)
                    .map(|edge| edge.neighbor)
                    .collect::<Vec<NodeId>>()
            });
        VectorCandidateSet::from_nodes(candidates)
    }
}
