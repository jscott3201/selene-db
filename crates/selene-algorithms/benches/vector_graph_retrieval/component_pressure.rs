//! Component-pool pressure rows for graph/vector retrieval benchmarks.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};

use crate::common::scale_label;

use super::support::{FACTS_PER_TOPIC, RESULT_K, basis_points, vector_scales};
use super::{MemoryRetrievalFixture, Query, RetrievalQuality};

const COMPONENT_PRESSURE_WIDTHS: &[usize] = &[1, 4, 16, 64];

pub(super) fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_component_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build(scale);
        for &width in COMPONENT_PRESSURE_WIDTHS {
            let actual_width = fixture.component_pool_width(width);
            let avg_candidates = fixture.average_component_pool_candidates(width);
            let quality = fixture.component_pressure_quality(width);
            group.throughput(Throughput::Elements(
                (fixture.query_count() * avg_candidates) as u64,
            ));
            group.bench_function(
                BenchmarkId::new(
                    format!("component_pool_w{actual_width}"),
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
                        black_box(fixture.component_pressure_total_coverage(width));
                    });
                },
            );
        }
    }
    group.finish();
}

impl MemoryRetrievalFixture {
    fn component_pressure_quality(&self, width: usize) -> RetrievalQuality {
        self.queries
            .iter()
            .map(|query| self.component_pressure_query_quality(query, width))
            .fold(RetrievalQuality::default(), |mut total, next| {
                total.coverage += next.coverage;
                total.current_coverage += next.current_coverage;
                total.precision += next.precision;
                total
            })
    }

    fn component_pressure_total_coverage(&self, width: usize) -> usize {
        self.component_pressure_quality(width).coverage
    }

    fn component_pressure_query_quality(&self, query: &Query, width: usize) -> RetrievalQuality {
        let selected = self.select_component_pool(query, width);
        self.selected_quality(query, selected)
    }

    fn select_component_pool(&self, query: &Query, width: usize) -> Vec<selene_core::NodeId> {
        let candidates = self.component_pool_candidates(query, width);
        let hits = self.score_candidate_ids(query, candidates);
        self.select_from_candidates(query, hits, true, false, true)
    }

    fn component_pool_candidates(&self, query: &Query, width: usize) -> Vec<selene_core::NodeId> {
        let Some(start) = self.component_offsets.get(&query.component).copied() else {
            return Vec::new();
        };
        let mut candidates = Vec::with_capacity(self.component_pool_candidate_count(query, width));
        for offset in 0..self.component_pool_width(width) {
            let component = self.component_order[(start + offset) % self.component_order.len()];
            if let Some(nodes) = self.component_candidates.get(&component) {
                candidates.extend(nodes.iter().copied());
            }
        }
        candidates
    }

    fn component_pool_width(&self, width: usize) -> usize {
        width.max(1).min(self.component_order.len())
    }

    fn component_pool_candidate_count(&self, query: &Query, width: usize) -> usize {
        let Some(start) = self.component_offsets.get(&query.component).copied() else {
            return 0;
        };
        (0..self.component_pool_width(width))
            .map(|offset| {
                let component = self.component_order[(start + offset) % self.component_order.len()];
                self.component_candidates
                    .get(&component)
                    .map_or(0, Vec::len)
            })
            .sum()
    }

    fn average_component_pool_candidates(&self, width: usize) -> usize {
        self.queries
            .iter()
            .map(|query| self.component_pool_candidate_count(query, width))
            .sum::<usize>()
            .checked_div(self.query_count())
            .unwrap_or(0)
    }
}
