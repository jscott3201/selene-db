use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::{CancellationChecker, NodeId, VectorMetric};
use selene_graph::{ApproximateVectorSearchOptions, VectorCandidateSet};

use super::{
    K, PRODUCTION_DIMENSIONS, PRODUCTION_SEARCH_WIDTH, ProductionDimensionFixture, compact_count,
};

const SPARSE_CANDIDATE_LEN: usize = 64;

pub(super) fn bench_production_turbo_quant_mixed_filtered_batch_dimension_projection(
    c: &mut Criterion,
) {
    let mut group =
        c.benchmark_group("graph_turbo_quant_production_mixed_filtered_batch_dimension_projection");
    for dimension in PRODUCTION_DIMENSIONS {
        let fixture = MixedFilterFixture::build(dimension);
        group.throughput(Throughput::Elements(fixture.candidate_count() as u64));
        group.bench_function(
            BenchmarkId::new(
                "cluster_cos",
                format!(
                    "tqcos_mixed_filtered_batch_c{PRODUCTION_SEARCH_WIDTH}_d{dimension}_q{}_cand{}_k{K}_recallbp{}_{}",
                    fixture.query_count(),
                    fixture.candidate_suffix(),
                    fixture.recall_basis_points(),
                    fixture.production.memory_suffix()
                ),
            ),
            |b| {
                b.iter(|| {
                    std::hint::black_box(fixture.total_overlap());
                });
            },
        );
    }
    group.finish();
}

pub(super) fn bench_production_turbo_quant_sparse_filtered_batch_dimension_projection(
    c: &mut Criterion,
) {
    let mut group = c
        .benchmark_group("graph_turbo_quant_production_sparse_filtered_batch_dimension_projection");
    for dimension in PRODUCTION_DIMENSIONS {
        let fixture = SparseFilterFixture::build(dimension);
        group.throughput(Throughput::Elements(fixture.candidate_count() as u64));
        group.bench_function(
            BenchmarkId::new(
                "cluster_cos",
                format!(
                    "tqcos_sparse_filtered_batch_c{PRODUCTION_SEARCH_WIDTH}_d{dimension}_q{}_cand{}_k{K}_recallbp{}_{}",
                    fixture.query_count(),
                    compact_count(SPARSE_CANDIDATE_LEN),
                    fixture.recall_basis_points(),
                    fixture.production.memory_suffix()
                ),
            ),
            |b| {
                b.iter(|| {
                    std::hint::black_box(fixture.total_overlap());
                });
            },
        );
    }
    group.finish();
}

struct MixedFilterFixture {
    production: ProductionDimensionFixture,
    candidate_sets: Vec<VectorCandidateSet>,
}

struct SparseFilterFixture {
    production: ProductionDimensionFixture,
    candidate_sets: Vec<VectorCandidateSet>,
}

impl MixedFilterFixture {
    fn build(dimension: usize) -> Self {
        let production = ProductionDimensionFixture::build(dimension);
        let candidate_sets = mixed_candidate_sets(&production);
        Self {
            production,
            candidate_sets,
        }
    }

    fn query_count(&self) -> usize {
        self.production.query_count()
    }

    fn candidate_count(&self) -> usize {
        self.candidate_sets
            .iter()
            .map(VectorCandidateSet::len)
            .sum()
    }

    fn candidate_suffix(&self) -> String {
        format!(
            "{}plus{}",
            compact_count(self.production.filtered_candidate_count()),
            compact_count(SPARSE_CANDIDATE_LEN)
        )
    }

    fn recall_basis_points(&self) -> usize {
        self.total_overlap() * 10_000 / (self.query_count() * K)
    }

    fn total_overlap(&self) -> usize {
        self.ids()
            .iter()
            .zip(&self.production.exact)
            .map(|(approx, exact)| exact.iter().filter(|id| approx.contains(id)).count())
            .sum()
    }

    fn ids(&self) -> Vec<Vec<usize>> {
        self.production
            .graph
            .approximate_vector_search_candidate_sets_batch_checked(
                &self.production.label,
                &self.production.property,
                &self.production.queries,
                &self.candidate_sets,
                ApproximateVectorSearchOptions::new(
                    VectorMetric::Cosine,
                    K,
                    PRODUCTION_SEARCH_WIDTH,
                ),
                CancellationChecker::disabled(),
            )
            .expect("production mixed-filter TurboQuant batch search succeeds")
            .into_iter()
            .map(|hits| {
                hits.into_iter()
                    .map(|hit| {
                        *self
                            .production
                            .node_to_ordinal
                            .get(&hit.node_id)
                            .expect("search hit node was inserted by this fixture")
                    })
                    .collect()
            })
            .collect()
    }
}

impl SparseFilterFixture {
    fn build(dimension: usize) -> Self {
        let production = ProductionDimensionFixture::build(dimension);
        let candidate_sets = production
            .filtered_candidates
            .iter()
            .enumerate()
            .map(|(query_index, dense)| sparse_candidate_set(&production, query_index, dense))
            .collect();
        Self {
            production,
            candidate_sets,
        }
    }

    fn query_count(&self) -> usize {
        self.production.query_count()
    }

    fn candidate_count(&self) -> usize {
        self.candidate_sets
            .iter()
            .map(VectorCandidateSet::len)
            .sum()
    }

    fn recall_basis_points(&self) -> usize {
        self.total_overlap() * 10_000 / (self.query_count() * K)
    }

    fn total_overlap(&self) -> usize {
        self.ids()
            .iter()
            .zip(&self.production.exact)
            .map(|(approx, exact)| exact.iter().filter(|id| approx.contains(id)).count())
            .sum()
    }

    fn ids(&self) -> Vec<Vec<usize>> {
        self.production
            .graph
            .approximate_vector_search_candidate_sets_batch_checked(
                &self.production.label,
                &self.production.property,
                &self.production.queries,
                &self.candidate_sets,
                ApproximateVectorSearchOptions::new(
                    VectorMetric::Cosine,
                    K,
                    PRODUCTION_SEARCH_WIDTH,
                ),
                CancellationChecker::disabled(),
            )
            .expect("production sparse-filter TurboQuant batch search succeeds")
            .into_iter()
            .map(|hits| {
                hits.into_iter()
                    .map(|hit| {
                        *self
                            .production
                            .node_to_ordinal
                            .get(&hit.node_id)
                            .expect("search hit node was inserted by this fixture")
                    })
                    .collect()
            })
            .collect()
    }
}

fn mixed_candidate_sets(fixture: &ProductionDimensionFixture) -> Vec<VectorCandidateSet> {
    fixture
        .filtered_candidates
        .iter()
        .enumerate()
        .map(|(query_index, dense)| {
            if query_index.is_multiple_of(2) {
                dense.clone()
            } else {
                sparse_candidate_set(fixture, query_index, dense)
            }
        })
        .collect()
}

fn sparse_candidate_set(
    fixture: &ProductionDimensionFixture,
    query_index: usize,
    dense: &VectorCandidateSet,
) -> VectorCandidateSet {
    let mut nodes = fixture.exact[query_index]
        .iter()
        .copied()
        .filter_map(|ordinal| node_for_ordinal(fixture, ordinal))
        .collect::<Vec<_>>();
    for node in dense.as_nodes().iter().copied() {
        if nodes.len() >= SPARSE_CANDIDATE_LEN {
            break;
        }
        if !nodes.contains(&node) {
            nodes.push(node);
        }
    }
    VectorCandidateSet::from_nodes(nodes)
}

fn node_for_ordinal(fixture: &ProductionDimensionFixture, ordinal: usize) -> Option<NodeId> {
    fixture
        .node_to_ordinal
        .iter()
        .find_map(|(node_id, candidate_ordinal)| {
            (*candidate_ordinal == ordinal).then_some(*node_id)
        })
}
