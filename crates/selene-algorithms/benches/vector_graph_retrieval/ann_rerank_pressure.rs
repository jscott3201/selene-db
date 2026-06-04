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

const ANN_GRAPH_FALLBACK_STRATEGIES: &[AnnGraphFallbackStrategy] = &[
    AnnGraphFallbackStrategy::GraphScopeCandidateSet,
    AnnGraphFallbackStrategy::LabelPropagationCandidateSet,
    AnnGraphFallbackStrategy::AnnWideHitSet,
    AnnGraphFallbackStrategy::AnnWideLabelUnion,
];

const ANN_BROAD_GRAPH_GATE_STRATEGIES: &[AnnBroadGraphGateStrategy] = &[
    AnnBroadGraphGateStrategy::GraphSessionCandidateSet,
    AnnBroadGraphGateStrategy::GraphSessionUnresolvedCandidateSet,
    AnnBroadGraphGateStrategy::AnnWideHitSet,
    AnnBroadGraphGateStrategy::AnnWideSessionIntersection,
    AnnBroadGraphGateStrategy::AnnBroadSessionIntersection,
    AnnBroadGraphGateStrategy::AnnWideSessionUnion,
    AnnBroadGraphGateStrategy::AnnBroadSessionUnion,
];

const BROAD_GATE_SEED_K: usize = 64;

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

#[derive(Clone, Copy)]
enum AnnGraphFallbackStrategy {
    GraphScopeCandidateSet,
    LabelPropagationCandidateSet,
    AnnWideHitSet,
    AnnWideLabelUnion,
}

impl AnnGraphFallbackStrategy {
    const fn name(self) -> &'static str {
        match self {
            Self::GraphScopeCandidateSet => "graph_scope_candidate_set_batch",
            Self::LabelPropagationCandidateSet => "label_propagation_candidate_set_batch",
            Self::AnnWideHitSet => "ann_wide_hit_set_batch_rerank",
            Self::AnnWideLabelUnion => "ann_wide_label_union_batch_rerank",
        }
    }
}

#[derive(Clone, Copy)]
enum AnnBroadGraphGateStrategy {
    GraphSessionCandidateSet,
    GraphSessionUnresolvedCandidateSet,
    AnnWideHitSet,
    AnnWideSessionIntersection,
    AnnBroadSessionIntersection,
    AnnWideSessionUnion,
    AnnBroadSessionUnion,
}

impl AnnBroadGraphGateStrategy {
    const fn name(self) -> &'static str {
        match self {
            Self::GraphSessionCandidateSet => "graph_session_candidate_set_batch",
            Self::GraphSessionUnresolvedCandidateSet => {
                "graph_session_unresolved_candidate_set_batch"
            }
            Self::AnnWideHitSet => "ann_wide_hit_set_batch_rerank",
            Self::AnnWideSessionIntersection => "ann_wide_session_intersection_batch_rerank",
            Self::AnnBroadSessionIntersection => "ann_broad_session_intersection_batch_rerank",
            Self::AnnWideSessionUnion => "ann_wide_session_union_batch_rerank",
            Self::AnnBroadSessionUnion => "ann_broad_session_union_batch_rerank",
        }
    }
}

pub(super) fn bench(c: &mut Criterion) {
    bench_ann_rerank_pressure(c);
    bench_ann_broad_graph_gate_pressure(c);
    bench_ann_graph_fallback_pressure(c);
}

fn bench_ann_rerank_pressure(c: &mut Criterion) {
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

fn bench_ann_broad_graph_gate_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_ann_broad_graph_gate_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build_with_topology(
            scale,
            TopologyNoise::NoisySparseMultiHopContradictedActiveHints,
        );
        for &strategy in ANN_BROAD_GRAPH_GATE_STRATEGIES {
            let avg_candidates = fixture.average_ann_broad_graph_gate_candidates(strategy);
            let quality = fixture.ann_broad_graph_gate_quality(strategy);
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
                        black_box(fixture.ann_broad_graph_gate_total_coverage(strategy));
                    });
                },
            );
        }
    }
    group.finish();
}

fn bench_ann_graph_fallback_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_ann_graph_fallback_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build_with_community_topology(
            scale,
            TopologyNoise::CrossTopicSupportRing,
        );
        for &strategy in ANN_GRAPH_FALLBACK_STRATEGIES {
            let avg_candidates = fixture.average_ann_graph_fallback_candidates(strategy);
            let quality = fixture.ann_graph_fallback_quality(strategy);
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
                        black_box(fixture.ann_graph_fallback_total_coverage(strategy));
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
        self.ann_hit_set(query, WIDE_SEED_K)
    }

    fn ann_hit_set(&self, query: &Query, k: usize) -> VectorCandidateSet {
        VectorCandidateSet::from_search_hits(self.ann_hits(query, k))
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

    fn ann_broad_graph_gate_quality(
        &self,
        strategy: AnnBroadGraphGateStrategy,
    ) -> RetrievalQuality {
        let mut queries = Vec::with_capacity(self.query_count());
        let mut candidate_sets = Vec::with_capacity(self.query_count());
        let mut max_candidates = 0;
        for query in &self.queries {
            let candidates = self.ann_broad_graph_gate_candidate_set(query, strategy);
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
            .expect("bench ANN broad graph gate candidate-set scoring succeeds");

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

    fn ann_broad_graph_gate_total_coverage(&self, strategy: AnnBroadGraphGateStrategy) -> usize {
        self.ann_broad_graph_gate_quality(strategy).coverage
    }

    fn average_ann_broad_graph_gate_candidates(
        &self,
        strategy: AnnBroadGraphGateStrategy,
    ) -> usize {
        self.queries
            .iter()
            .map(|query| {
                self.ann_broad_graph_gate_candidate_set(query, strategy)
                    .len()
            })
            .sum::<usize>()
            .checked_div(self.query_count())
            .unwrap_or(0)
    }

    fn ann_broad_graph_gate_candidate_set(
        &self,
        query: &Query,
        strategy: AnnBroadGraphGateStrategy,
    ) -> VectorCandidateSet {
        let session = match strategy {
            AnnBroadGraphGateStrategy::GraphSessionCandidateSet
            | AnnBroadGraphGateStrategy::AnnWideSessionIntersection
            | AnnBroadGraphGateStrategy::AnnBroadSessionIntersection
            | AnnBroadGraphGateStrategy::AnnWideSessionUnion
            | AnnBroadGraphGateStrategy::AnnBroadSessionUnion => {
                self.graph_session_candidate_set(query)
            }
            AnnBroadGraphGateStrategy::GraphSessionUnresolvedCandidateSet => {
                return self.graph_session_unresolved_candidate_set(query);
            }
            AnnBroadGraphGateStrategy::AnnWideHitSet => {
                return self.ann_wide_hit_set(query);
            }
        };
        match strategy {
            AnnBroadGraphGateStrategy::GraphSessionCandidateSet => session,
            AnnBroadGraphGateStrategy::AnnWideSessionIntersection => {
                self.ann_wide_hit_set(query).intersection(&session)
            }
            AnnBroadGraphGateStrategy::AnnBroadSessionIntersection => self
                .ann_hit_set(query, BROAD_GATE_SEED_K)
                .intersection(&session),
            AnnBroadGraphGateStrategy::AnnWideSessionUnion => {
                self.ann_wide_hit_set(query).union(&session)
            }
            AnnBroadGraphGateStrategy::AnnBroadSessionUnion => {
                self.ann_hit_set(query, BROAD_GATE_SEED_K).union(&session)
            }
            AnnBroadGraphGateStrategy::GraphSessionUnresolvedCandidateSet
            | AnnBroadGraphGateStrategy::AnnWideHitSet => unreachable!(),
        }
    }

    fn graph_session_unresolved_candidate_set(&self, query: &Query) -> VectorCandidateSet {
        self.graph_session_candidate_set(query)
            .intersection(&self.graph_unresolved_current_candidate_set)
    }

    fn graph_session_candidate_set(&self, query: &Query) -> VectorCandidateSet {
        let mut candidates = Vec::new();
        if let Some(anchor_edges) = self.graph.outgoing_edges(query.anchor) {
            for session_edge in anchor_edges
                .iter()
                .filter(|edge| edge.label == self.session_edge)
            {
                if let Some(session_members) = self.graph.incoming_edges(session_edge.neighbor) {
                    candidates.extend(
                        session_members
                            .iter()
                            .filter(|edge| edge.label == self.session_edge)
                            .map(|edge| edge.neighbor),
                    );
                }
            }
        }
        VectorCandidateSet::from_nodes(candidates)
    }

    fn ann_graph_fallback_quality(&self, strategy: AnnGraphFallbackStrategy) -> RetrievalQuality {
        let mut queries = Vec::with_capacity(self.query_count());
        let mut candidate_sets = Vec::with_capacity(self.query_count());
        let mut max_candidates = 0;
        for query in &self.queries {
            let candidates = self.ann_graph_fallback_candidate_set(query, strategy);
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
            .expect("bench ANN graph fallback candidate-set scoring succeeds");

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

    fn ann_graph_fallback_total_coverage(&self, strategy: AnnGraphFallbackStrategy) -> usize {
        self.ann_graph_fallback_quality(strategy).coverage
    }

    fn average_ann_graph_fallback_candidates(&self, strategy: AnnGraphFallbackStrategy) -> usize {
        self.queries
            .iter()
            .map(|query| self.ann_graph_fallback_candidate_set(query, strategy).len())
            .sum::<usize>()
            .checked_div(self.query_count())
            .unwrap_or(0)
    }

    fn ann_graph_fallback_candidate_set(
        &self,
        query: &Query,
        strategy: AnnGraphFallbackStrategy,
    ) -> VectorCandidateSet {
        match strategy {
            AnnGraphFallbackStrategy::GraphScopeCandidateSet => {
                self.graph_scope_candidate_set(query)
            }
            AnnGraphFallbackStrategy::LabelPropagationCandidateSet => {
                self.label_propagation_candidate_set(query)
            }
            AnnGraphFallbackStrategy::AnnWideHitSet => self.ann_wide_hit_set(query),
            AnnGraphFallbackStrategy::AnnWideLabelUnion => self
                .ann_wide_hit_set(query)
                .union(&self.label_propagation_candidate_set(query)),
        }
    }

    fn label_propagation_candidate_set(&self, query: &Query) -> VectorCandidateSet {
        let candidates = self
            .label_by_node
            .get(&query.anchor)
            .and_then(|community| self.label_candidates.get(community))
            .into_iter()
            .flat_map(|candidates| candidates.iter().copied());
        VectorCandidateSet::from_nodes(candidates)
    }

    fn graph_scope_candidate_set(&self, query: &Query) -> VectorCandidateSet {
        let mut candidates = Vec::new();
        if let Some(anchor_edges) = self.graph.outgoing_edges(query.anchor) {
            for scope_edge in anchor_edges
                .iter()
                .filter(|edge| edge.label == self.scope_edge)
            {
                if let Some(scope_members) = self.graph.incoming_edges(scope_edge.neighbor) {
                    candidates.extend(
                        scope_members
                            .iter()
                            .filter(|edge| edge.label == self.scope_edge)
                            .map(|edge| edge.neighbor),
                    );
                }
            }
        }
        VectorCandidateSet::from_nodes(candidates)
    }
}
