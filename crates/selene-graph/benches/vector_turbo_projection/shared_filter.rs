use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::{CancellationChecker, VectorMetric};
use selene_graph::ApproximateVectorSearchOptions;

use super::{
    K, PRODUCTION_DIMENSIONS, PRODUCTION_SEARCH_WIDTH, ProductionDimensionFixture, compact_count,
};

pub(super) fn bench_production_turbo_quant_shared_filtered_batch_dimension_projection(
    c: &mut Criterion,
) {
    let mut group = c
        .benchmark_group("graph_turbo_quant_production_shared_filtered_batch_dimension_projection");
    for dimension in PRODUCTION_DIMENSIONS {
        let fixture = ProductionDimensionFixture::build(dimension);
        group.throughput(Throughput::Elements(
            (fixture.filtered_candidate_count() * fixture.query_count()) as u64,
        ));
        group.bench_function(
            BenchmarkId::new(
                "cluster_cos",
                format!(
                    "tqcos_shared_filtered_batch_c{PRODUCTION_SEARCH_WIDTH}_d{dimension}_q{}_cand{}_k{K}_recallbp{}_{}",
                    fixture.query_count(),
                    compact_count(fixture.filtered_candidate_count()),
                    shared_filtered_batch_recall_basis_points(&fixture),
                    fixture.memory_suffix()
                ),
            ),
            |b| {
                b.iter(|| {
                    std::hint::black_box(shared_filtered_batch_total_overlap(&fixture));
                });
            },
        );
    }
    group.finish();
}

fn shared_filtered_batch_recall_basis_points(fixture: &ProductionDimensionFixture) -> usize {
    shared_filtered_batch_total_overlap(fixture) * 10_000 / (fixture.query_count() * K)
}

fn shared_filtered_batch_total_overlap(fixture: &ProductionDimensionFixture) -> usize {
    shared_filtered_batch_ids(fixture)
        .iter()
        .zip(&fixture.exact)
        .map(|(approx, exact)| exact.iter().filter(|id| approx.contains(id)).count())
        .sum()
}

fn shared_filtered_batch_ids(fixture: &ProductionDimensionFixture) -> Vec<Vec<usize>> {
    let shared_candidates = fixture
        .filtered_candidates
        .first()
        .expect("production fixture has at least one filtered candidate set");
    let candidate_sets = vec![shared_candidates.clone(); fixture.query_count()];
    fixture
        .graph
        .approximate_vector_search_candidate_sets_batch_checked(
            &fixture.label,
            &fixture.property,
            &fixture.queries,
            &candidate_sets,
            ApproximateVectorSearchOptions::new(VectorMetric::Cosine, K, PRODUCTION_SEARCH_WIDTH),
            CancellationChecker::disabled(),
        )
        .expect("production shared-filter TurboQuant batch search succeeds")
        .into_iter()
        .map(|hits| {
            hits.into_iter()
                .map(|hit| {
                    *fixture
                        .node_to_ordinal
                        .get(&hit.node_id)
                        .expect("search hit node was inserted by this fixture")
                })
                .collect()
        })
        .collect()
}
