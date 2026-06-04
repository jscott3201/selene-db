//! Community-partition pressure rows for graph/vector retrieval benchmarks.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::NodeId;

use crate::common::scale_label;

use super::support::{FACTS_PER_TOPIC, RESULT_K, basis_points, vector_scales};
use super::{MemoryRetrievalFixture, Query, RetrievalQuality, TopologyNoise};

const COMMUNITY_STRATEGIES: &[CommunityStrategy] = &[
    CommunityStrategy::NoisyWcc,
    CommunityStrategy::Louvain,
    CommunityStrategy::LabelPropagation,
    CommunityStrategy::TopicFilter,
];

#[derive(Clone, Copy, Debug)]
enum CommunityStrategy {
    NoisyWcc,
    Louvain,
    LabelPropagation,
    TopicFilter,
}

impl CommunityStrategy {
    const fn name(self) -> &'static str {
        match self {
            Self::NoisyWcc => "noisy_wcc",
            Self::Louvain => "louvain",
            Self::LabelPropagation => "label_propagation",
            Self::TopicFilter => "topic_filter",
        }
    }
}

pub(super) fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_community_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build_with_community_topology(
            scale,
            TopologyNoise::CrossTopicSupportRing,
        );
        for &strategy in COMMUNITY_STRATEGIES {
            let avg_candidates = fixture.average_community_candidates(strategy);
            let quality = fixture.community_quality(strategy);
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
                        black_box(fixture.community_total_coverage(strategy));
                    });
                },
            );
        }
    }
    group.finish();
}

impl MemoryRetrievalFixture {
    fn community_quality(&self, strategy: CommunityStrategy) -> RetrievalQuality {
        self.queries
            .iter()
            .map(|query| self.community_query_quality(query, strategy))
            .fold(RetrievalQuality::default(), |mut total, next| {
                total.coverage += next.coverage;
                total.current_coverage += next.current_coverage;
                total.precision += next.precision;
                total
            })
    }

    fn community_total_coverage(&self, strategy: CommunityStrategy) -> usize {
        self.community_quality(strategy).coverage
    }

    fn community_query_quality(
        &self,
        query: &Query,
        strategy: CommunityStrategy,
    ) -> RetrievalQuality {
        let selected = self.select_community_candidates(query, strategy);
        self.selected_quality(query, selected)
    }

    fn select_community_candidates(
        &self,
        query: &Query,
        strategy: CommunityStrategy,
    ) -> Vec<NodeId> {
        let candidates = self.community_candidates(query, strategy);
        let hits = self.score_candidate_ids(query, candidates);
        self.select_from_candidates(query, hits, true, false, true)
    }

    fn community_candidates(&self, query: &Query, strategy: CommunityStrategy) -> Vec<NodeId> {
        match strategy {
            CommunityStrategy::NoisyWcc => self
                .component_candidates
                .get(&query.component)
                .cloned()
                .unwrap_or_default(),
            CommunityStrategy::Louvain => self
                .louvain_by_node
                .get(&query.anchor)
                .and_then(|community| self.louvain_candidates.get(community))
                .cloned()
                .unwrap_or_default(),
            CommunityStrategy::LabelPropagation => self
                .label_by_node
                .get(&query.anchor)
                .and_then(|community| self.label_candidates.get(community))
                .cloned()
                .unwrap_or_default(),
            CommunityStrategy::TopicFilter => self
                .topic_candidates
                .get(query.topic)
                .cloned()
                .unwrap_or_default(),
        }
    }

    fn community_candidate_count(&self, query: &Query, strategy: CommunityStrategy) -> usize {
        match strategy {
            CommunityStrategy::NoisyWcc => self
                .component_candidates
                .get(&query.component)
                .map_or(0, Vec::len),
            CommunityStrategy::Louvain => self
                .louvain_by_node
                .get(&query.anchor)
                .and_then(|community| self.louvain_candidates.get(community))
                .map_or(0, Vec::len),
            CommunityStrategy::LabelPropagation => self
                .label_by_node
                .get(&query.anchor)
                .and_then(|community| self.label_candidates.get(community))
                .map_or(0, Vec::len),
            CommunityStrategy::TopicFilter => {
                self.topic_candidates.get(query.topic).map_or(0, Vec::len)
            }
        }
    }

    fn average_community_candidates(&self, strategy: CommunityStrategy) -> usize {
        self.queries
            .iter()
            .map(|query| self.community_candidate_count(query, strategy))
            .sum::<usize>()
            .checked_div(self.query_count())
            .unwrap_or(0)
    }
}
