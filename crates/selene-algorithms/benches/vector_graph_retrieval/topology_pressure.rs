//! Topology-noise pressure rows for graph/vector retrieval benchmarks.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::NodeId;

use crate::common::scale_label;

use super::support::{FACTS_PER_TOPIC, RESULT_K, basis_points, vector_scales};
use super::{MemoryRetrievalFixture, Query, RetrievalQuality, TopologyNoise};

const TOPOLOGY_STRATEGIES: &[TopologyStrategy] =
    &[TopologyStrategy::NoisyWcc, TopologyStrategy::TopicFilter];

#[derive(Clone, Copy, Debug)]
enum TopologyStrategy {
    NoisyWcc,
    TopicFilter,
}

impl TopologyStrategy {
    const fn name(self) -> &'static str {
        match self {
            Self::NoisyWcc => "noisy_wcc",
            Self::TopicFilter => "topic_filter",
        }
    }
}

pub(super) fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_topology_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build_with_topology(
            scale,
            TopologyNoise::CrossTopicSupportRing,
        );
        for &strategy in TOPOLOGY_STRATEGIES {
            let avg_candidates = fixture.average_topology_candidates(strategy);
            let quality = fixture.topology_quality(strategy);
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
                        black_box(fixture.topology_total_coverage(strategy));
                    });
                },
            );
        }
    }
    group.finish();
}

impl MemoryRetrievalFixture {
    fn topology_quality(&self, strategy: TopologyStrategy) -> RetrievalQuality {
        self.queries
            .iter()
            .map(|query| self.topology_query_quality(query, strategy))
            .fold(RetrievalQuality::default(), |mut total, next| {
                total.coverage += next.coverage;
                total.current_coverage += next.current_coverage;
                total.precision += next.precision;
                total
            })
    }

    fn topology_total_coverage(&self, strategy: TopologyStrategy) -> usize {
        self.topology_quality(strategy).coverage
    }

    fn topology_query_quality(
        &self,
        query: &Query,
        strategy: TopologyStrategy,
    ) -> RetrievalQuality {
        let selected = self.select_topology_candidates(query, strategy);
        self.selected_quality(query, selected)
    }

    fn select_topology_candidates(&self, query: &Query, strategy: TopologyStrategy) -> Vec<NodeId> {
        let candidates = self.topology_candidates(query, strategy);
        let hits = self.score_candidate_ids(query, candidates);
        self.select_from_candidates(query, hits, true, false, true)
    }

    fn topology_candidates(&self, query: &Query, strategy: TopologyStrategy) -> Vec<NodeId> {
        match strategy {
            TopologyStrategy::NoisyWcc => self
                .component_candidates
                .get(&query.component)
                .cloned()
                .unwrap_or_default(),
            TopologyStrategy::TopicFilter => self
                .topic_candidates
                .get(query.topic)
                .cloned()
                .unwrap_or_default(),
        }
    }

    fn topology_candidate_count(&self, query: &Query, strategy: TopologyStrategy) -> usize {
        match strategy {
            TopologyStrategy::NoisyWcc => self
                .component_candidates
                .get(&query.component)
                .map_or(0, Vec::len),
            TopologyStrategy::TopicFilter => {
                self.topic_candidates.get(query.topic).map_or(0, Vec::len)
            }
        }
    }

    fn average_topology_candidates(&self, strategy: TopologyStrategy) -> usize {
        self.queries
            .iter()
            .map(|query| self.topology_candidate_count(query, strategy))
            .sum::<usize>()
            .checked_div(self.query_count())
            .unwrap_or(0)
    }
}
