//! Task/session graph filter rows for graph/vector retrieval benchmarks.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::NodeId;

use crate::common::scale_label;

use super::support::{FACTS_PER_TOPIC, RESULT_K, basis_points, vector_scales};
use super::{MemoryRetrievalFixture, Query, RetrievalQuality, TopologyNoise};

const SESSION_STRATEGIES: &[SessionStrategy] = &[
    SessionStrategy::NoisyWcc,
    SessionStrategy::LabelPropagation,
    SessionStrategy::GraphSessionFilter,
    SessionStrategy::GraphScopeFilter,
    SessionStrategy::TopicFilter,
];

#[derive(Clone, Copy, Debug)]
enum SessionStrategy {
    NoisyWcc,
    LabelPropagation,
    GraphSessionFilter,
    GraphScopeFilter,
    TopicFilter,
}

impl SessionStrategy {
    const fn name(self) -> &'static str {
        match self {
            Self::NoisyWcc => "noisy_wcc",
            Self::LabelPropagation => "label_propagation",
            Self::GraphSessionFilter => "graph_session_filter",
            Self::GraphScopeFilter => "graph_scope_filter",
            Self::TopicFilter => "topic_filter",
        }
    }
}

pub(super) fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_session_filter_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build_with_community_topology(
            scale,
            TopologyNoise::CrossTopicSupportRing,
        );
        for &strategy in SESSION_STRATEGIES {
            let avg_candidates = fixture.average_session_candidates(strategy);
            let quality = fixture.session_quality(strategy);
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
                        black_box(fixture.session_total_coverage(strategy));
                    });
                },
            );
        }
    }
    group.finish();
}

impl MemoryRetrievalFixture {
    fn session_quality(&self, strategy: SessionStrategy) -> RetrievalQuality {
        self.queries
            .iter()
            .map(|query| self.session_query_quality(query, strategy))
            .fold(RetrievalQuality::default(), |mut total, next| {
                total.coverage += next.coverage;
                total.current_coverage += next.current_coverage;
                total.precision += next.precision;
                total
            })
    }

    fn session_total_coverage(&self, strategy: SessionStrategy) -> usize {
        self.session_quality(strategy).coverage
    }

    fn session_query_quality(&self, query: &Query, strategy: SessionStrategy) -> RetrievalQuality {
        let selected = self.select_session_candidates(query, strategy);
        self.selected_quality(query, selected)
    }

    fn select_session_candidates(&self, query: &Query, strategy: SessionStrategy) -> Vec<NodeId> {
        let candidates = self.session_candidates(query, strategy);
        let hits = self.score_candidate_ids(query, candidates);
        self.select_from_candidates(query, hits, true, false, true)
    }

    fn session_candidates(&self, query: &Query, strategy: SessionStrategy) -> Vec<NodeId> {
        match strategy {
            SessionStrategy::NoisyWcc => self
                .component_candidates
                .get(&query.component)
                .cloned()
                .unwrap_or_default(),
            SessionStrategy::LabelPropagation => self
                .label_by_node
                .get(&query.anchor)
                .and_then(|community| self.label_candidates.get(community))
                .cloned()
                .unwrap_or_default(),
            SessionStrategy::GraphSessionFilter => self.graph_session_candidates(query),
            SessionStrategy::GraphScopeFilter => self.graph_session_scope_candidates(query),
            SessionStrategy::TopicFilter => self
                .topic_candidates
                .get(query.topic)
                .cloned()
                .unwrap_or_default(),
        }
    }

    fn graph_session_candidates(&self, query: &Query) -> Vec<NodeId> {
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
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }

    fn graph_session_scope_candidates(&self, query: &Query) -> Vec<NodeId> {
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
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }

    fn session_candidate_count(&self, query: &Query, strategy: SessionStrategy) -> usize {
        self.session_candidates(query, strategy).len()
    }

    fn average_session_candidates(&self, strategy: SessionStrategy) -> usize {
        self.queries
            .iter()
            .map(|query| self.session_candidate_count(query, strategy))
            .sum::<usize>()
            .checked_div(self.query_count())
            .unwrap_or(0)
    }
}
