//! Query-derived graph filter rows for graph/vector retrieval benchmarks.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::NodeId;

use crate::common::scale_label;

use super::support::{FACTS_PER_TOPIC, RESULT_K, basis_points, vector_scales};
use super::{MemoryRetrievalFixture, Query, RetrievalQuality, TopologyNoise};

const QUERY_FILTER_STRATEGIES: &[QueryFilterStrategy] = &[
    QueryFilterStrategy::NoisyWcc,
    QueryFilterStrategy::LabelPropagation,
    QueryFilterStrategy::GraphScopeFilter,
    QueryFilterStrategy::TopicFilter,
];

#[derive(Clone, Copy, Debug)]
enum QueryFilterStrategy {
    NoisyWcc,
    LabelPropagation,
    GraphScopeFilter,
    TopicFilter,
}

impl QueryFilterStrategy {
    const fn name(self) -> &'static str {
        match self {
            Self::NoisyWcc => "noisy_wcc",
            Self::LabelPropagation => "label_propagation",
            Self::GraphScopeFilter => "graph_scope_filter",
            Self::TopicFilter => "topic_filter",
        }
    }
}

pub(super) fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_query_filter_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build_with_community_topology(
            scale,
            TopologyNoise::CrossTopicSupportRing,
        );
        for &strategy in QUERY_FILTER_STRATEGIES {
            let avg_candidates = fixture.average_query_filter_candidates(strategy);
            let quality = fixture.query_filter_quality(strategy);
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
                        black_box(fixture.query_filter_total_coverage(strategy));
                    });
                },
            );
        }
    }
    group.finish();
}

impl MemoryRetrievalFixture {
    fn query_filter_quality(&self, strategy: QueryFilterStrategy) -> RetrievalQuality {
        self.queries
            .iter()
            .map(|query| self.query_filter_query_quality(query, strategy))
            .fold(RetrievalQuality::default(), |mut total, next| {
                total.coverage += next.coverage;
                total.current_coverage += next.current_coverage;
                total.precision += next.precision;
                total
            })
    }

    fn query_filter_total_coverage(&self, strategy: QueryFilterStrategy) -> usize {
        self.query_filter_quality(strategy).coverage
    }

    fn query_filter_query_quality(
        &self,
        query: &Query,
        strategy: QueryFilterStrategy,
    ) -> RetrievalQuality {
        let selected = self.select_query_filter_candidates(query, strategy);
        self.selected_quality(query, selected)
    }

    fn select_query_filter_candidates(
        &self,
        query: &Query,
        strategy: QueryFilterStrategy,
    ) -> Vec<NodeId> {
        let candidates = self.query_filter_candidates(query, strategy);
        let hits = self.score_candidate_ids(query, candidates);
        self.select_from_candidates(query, hits, true, false, true)
    }

    fn query_filter_candidates(&self, query: &Query, strategy: QueryFilterStrategy) -> Vec<NodeId> {
        match strategy {
            QueryFilterStrategy::NoisyWcc => self
                .component_candidates
                .get(&query.component)
                .cloned()
                .unwrap_or_default(),
            QueryFilterStrategy::LabelPropagation => self
                .label_by_node
                .get(&query.anchor)
                .and_then(|community| self.label_candidates.get(community))
                .cloned()
                .unwrap_or_default(),
            QueryFilterStrategy::GraphScopeFilter => self.graph_scope_candidates(query),
            QueryFilterStrategy::TopicFilter => self
                .topic_candidates
                .get(query.topic)
                .cloned()
                .unwrap_or_default(),
        }
    }

    fn graph_scope_candidates(&self, query: &Query) -> Vec<NodeId> {
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

    fn query_filter_candidate_count(&self, query: &Query, strategy: QueryFilterStrategy) -> usize {
        self.query_filter_candidates(query, strategy).len()
    }

    fn average_query_filter_candidates(&self, strategy: QueryFilterStrategy) -> usize {
        self.queries
            .iter()
            .map(|query| self.query_filter_candidate_count(query, strategy))
            .sum::<usize>()
            .checked_div(self.query_count())
            .unwrap_or(0)
    }
}
