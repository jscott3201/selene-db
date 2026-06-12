#![allow(missing_docs)]
#![allow(dead_code)]
//! Criterion benches for TurboQuant storage/search across embedding dimensions.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;
#[path = "vector_pq_support/turbo_quant.rs"]
mod turbo_quant;

use std::collections::HashMap;
use std::mem::size_of;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{
    CancellationChecker, GraphId, LabelSet, NodeId, PropertyMap, Value, VectorMetric, VectorValue,
    db_string, exact_vector_top_k,
};
use selene_graph::{
    ApproximateVectorSearchOptions, SharedGraph, VectorCandidateSet, VectorIndexKind,
};
use turbo_quant::{
    TurboQuantCalibration, TurboQuantCodebook, TurboQuantIndex, TurboQuantScorer, TurboQuantVariant,
};

const ROWS: usize = 10_000;
const QUERY_COUNT: usize = 8;
const K: usize = 10;
const PRODUCTION_SEARCH_WIDTH: usize = 512;
const FILTERED_CANDIDATE_LEN: usize = 4_096;
const DIMENSIONS: [usize; 3] = [128, 768, 1536];
const VARIANT: TurboQuantVariant = TurboQuantVariant {
    name: "tqplus4lut_c1024",
    bit_width: 4,
    candidates: 1024,
    codebook: TurboQuantCodebook::NormalLloydMax,
    calibration: TurboQuantCalibration::Quantile,
    scorer: TurboQuantScorer::ByteLut,
};
const BLOCKED_VARIANT: TurboQuantVariant = TurboQuantVariant {
    name: "tqplus4blocked_c1024",
    bit_width: 4,
    candidates: 1024,
    codebook: TurboQuantCodebook::NormalLloydMax,
    calibration: TurboQuantCalibration::Quantile,
    scorer: TurboQuantScorer::BlockedByteLut,
};
const BLOCKED_WIDE_VARIANT: TurboQuantVariant = TurboQuantVariant {
    name: "tqplus4blockedwide_c1024",
    bit_width: 4,
    candidates: 1024,
    codebook: TurboQuantCodebook::NormalLloydMax,
    calibration: TurboQuantCalibration::Quantile,
    scorer: TurboQuantScorer::BlockedWideByteLut,
};
const BLOCKED_FAST_SCAN_VARIANT: TurboQuantVariant = TurboQuantVariant {
    name: "tqplus4fastscan_c1024",
    bit_width: 4,
    candidates: 1024,
    codebook: TurboQuantCodebook::NormalLloydMax,
    calibration: TurboQuantCalibration::Quantile,
    scorer: TurboQuantScorer::BlockedFastScanLut,
};

fn bench_turbo_quant_dimension_projection(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_turbo_quant_dimension_projection");
    for dimension in DIMENSIONS {
        let fixture = DimensionFixture::build(dimension, VARIANT);
        group.throughput(Throughput::Elements(
            (fixture.rows() * fixture.query_count()) as u64,
        ));
        group.bench_function(
            BenchmarkId::new(
                "cluster_cos",
                format!(
                    "{}_d{dimension}_n{}_k{K}_recallbp{}_{}",
                    VARIANT.name,
                    compact_count(fixture.rows()),
                    fixture.recall_basis_points(),
                    fixture.memory_suffix()
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

fn bench_blocked_turbo_quant_dimension_projection(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_turbo_quant_blocked_dimension_projection");
    for dimension in DIMENSIONS {
        let fixture = DimensionFixture::build(dimension, BLOCKED_VARIANT);
        group.throughput(Throughput::Elements(
            (fixture.rows() * fixture.query_count()) as u64,
        ));
        group.bench_function(
            BenchmarkId::new(
                "cluster_cos",
                format!(
                    "{}_d{dimension}_n{}_k{K}_recallbp{}_{}",
                    BLOCKED_VARIANT.name,
                    compact_count(fixture.rows()),
                    fixture.recall_basis_points(),
                    fixture.memory_suffix()
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

fn bench_blocked_wide_turbo_quant_dimension_projection(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_turbo_quant_blocked_wide_dimension_projection");
    for dimension in DIMENSIONS {
        let fixture = DimensionFixture::build(dimension, BLOCKED_WIDE_VARIANT);
        group.throughput(Throughput::Elements(
            (fixture.rows() * fixture.query_count()) as u64,
        ));
        group.bench_function(
            BenchmarkId::new(
                "cluster_cos",
                format!(
                    "{}_d{dimension}_n{}_k{K}_recallbp{}_{}",
                    BLOCKED_WIDE_VARIANT.name,
                    compact_count(fixture.rows()),
                    fixture.recall_basis_points(),
                    fixture.memory_suffix()
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

fn bench_blocked_fast_scan_turbo_quant_dimension_projection(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_turbo_quant_blocked_fast_scan_dimension_projection");
    for dimension in DIMENSIONS {
        let fixture = DimensionFixture::build(dimension, BLOCKED_FAST_SCAN_VARIANT);
        group.throughput(Throughput::Elements(
            (fixture.rows() * fixture.query_count()) as u64,
        ));
        group.bench_function(
            BenchmarkId::new(
                "cluster_cos",
                format!(
                    "{}_d{dimension}_n{}_k{K}_recallbp{}_{}",
                    BLOCKED_FAST_SCAN_VARIANT.name,
                    compact_count(fixture.rows()),
                    fixture.recall_basis_points(),
                    fixture.memory_suffix()
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

fn bench_production_turbo_quant_dimension_projection(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_turbo_quant_production_dimension_projection");
    for dimension in DIMENSIONS {
        let fixture = ProductionDimensionFixture::build(dimension);
        group.throughput(Throughput::Elements(
            (fixture.rows() * fixture.query_count()) as u64,
        ));
        group.bench_function(
            BenchmarkId::new(
                "cluster_cos",
                format!(
                    "tqcos_c{PRODUCTION_SEARCH_WIDTH}_d{dimension}_n{}_k{K}_recallbp{}_{}",
                    compact_count(fixture.rows()),
                    fixture.recall_basis_points(),
                    fixture.memory_suffix()
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

fn bench_production_turbo_quant_batch_dimension_projection(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_turbo_quant_production_batch_dimension_projection");
    for dimension in DIMENSIONS {
        let fixture = ProductionDimensionFixture::build(dimension);
        group.throughput(Throughput::Elements(
            (fixture.rows() * fixture.query_count()) as u64,
        ));
        group.bench_function(
            BenchmarkId::new(
                "cluster_cos",
                format!(
                    "tqcos_batch_c{PRODUCTION_SEARCH_WIDTH}_d{dimension}_q{QUERY_COUNT}_n{}_k{K}_recallbp{}_{}",
                    compact_count(fixture.rows()),
                    fixture.batch_recall_basis_points(),
                    fixture.memory_suffix()
                ),
            ),
            |b| {
                b.iter(|| {
                    std::hint::black_box(fixture.total_batch_overlap());
                });
            },
        );
    }
    group.finish();
}

fn bench_production_turbo_quant_filtered_dimension_projection(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_turbo_quant_production_filtered_dimension_projection");
    for dimension in DIMENSIONS {
        let fixture = ProductionDimensionFixture::build(dimension);
        group.throughput(Throughput::Elements(
            (fixture.filtered_candidate_count() * fixture.query_count()) as u64,
        ));
        group.bench_function(
            BenchmarkId::new(
                "cluster_cos",
                format!(
                    "tqcos_filtered_c{PRODUCTION_SEARCH_WIDTH}_d{dimension}_q{QUERY_COUNT}_cand{}_k{K}_recallbp{}_{}",
                    compact_count(fixture.filtered_candidate_count()),
                    fixture.filtered_recall_basis_points(),
                    fixture.memory_suffix()
                ),
            ),
            |b| {
                b.iter(|| {
                    std::hint::black_box(fixture.filtered_total_overlap());
                });
            },
        );
    }
    group.finish();
}

fn bench_production_turbo_quant_filtered_batch_dimension_projection(c: &mut Criterion) {
    let mut group =
        c.benchmark_group("graph_turbo_quant_production_filtered_batch_dimension_projection");
    for dimension in DIMENSIONS {
        let fixture = ProductionDimensionFixture::build(dimension);
        group.throughput(Throughput::Elements(
            (fixture.filtered_candidate_count() * fixture.query_count()) as u64,
        ));
        group.bench_function(
            BenchmarkId::new(
                "cluster_cos",
                format!(
                    "tqcos_filtered_batch_c{PRODUCTION_SEARCH_WIDTH}_d{dimension}_q{QUERY_COUNT}_cand{}_k{K}_recallbp{}_{}",
                    compact_count(fixture.filtered_candidate_count()),
                    fixture.filtered_batch_recall_basis_points(),
                    fixture.memory_suffix()
                ),
            ),
            |b| {
                b.iter(|| {
                    std::hint::black_box(fixture.filtered_batch_total_overlap());
                });
            },
        );
    }
    group.finish();
}

#[derive(Debug)]
struct DimensionFixture {
    dimension: usize,
    vectors: Vec<VectorValue>,
    queries: Vec<VectorValue>,
    exact: Vec<Vec<usize>>,
    turbo: TurboQuantIndex,
}

struct ProductionDimensionFixture {
    dimension: usize,
    graph: SharedGraph,
    label: selene_core::DbString,
    property: selene_core::DbString,
    queries: Vec<VectorValue>,
    exact: Vec<Vec<usize>>,
    node_to_ordinal: HashMap<NodeId, usize>,
    filtered_candidates: Vec<VectorCandidateSet>,
}

impl ProductionDimensionFixture {
    fn build(dimension: usize) -> Self {
        let label = db_string("TurboProjectionDoc").expect("bench label is valid");
        let property = db_string("embedding").expect("bench property is valid");
        let graph = SharedGraph::new(GraphId::new(95_000 + dimension as u64));
        let vectors = (0..ROWS)
            .map(|seed| dimension_vector(seed, dimension, 0.0))
            .collect::<Vec<_>>();
        let mut node_to_ordinal = HashMap::with_capacity(vectors.len());
        let mut ordinal_to_node = Vec::with_capacity(vectors.len());
        {
            let mut txn = graph.begin_write();
            let mut mutator = txn.mutator();
            for (ordinal, vector) in vectors.iter().enumerate() {
                let props =
                    PropertyMap::from_pairs([(property.clone(), Value::Vector(vector.clone()))])
                        .expect("bench vector property map is valid");
                let node_id = mutator
                    .create_node(LabelSet::single(label.clone()), props)
                    .expect("bench vector node insert succeeds");
                node_to_ordinal.insert(node_id, ordinal);
                ordinal_to_node.push(node_id);
            }
            txn.commit().expect("bench vector fixture commit succeeds");
        }
        graph
            .create_vector_index(
                label.clone(),
                property.clone(),
                VectorIndexKind::TurboQuantCosine,
                dimension
                    .try_into()
                    .expect("bench vector dimension fits u32"),
            )
            .expect("bench TurboQuant vector index builds");
        let query_seeds = (0..QUERY_COUNT)
            .map(|idx| {
                let cluster = idx % cluster_count();
                (cluster + (ROWS / cluster_count() / 2) * cluster_count()).min(ROWS - 1)
            })
            .collect::<Vec<_>>();
        let queries = query_seeds
            .iter()
            .copied()
            .map(|seed| dimension_vector(seed, dimension, 0.0003))
            .collect::<Vec<_>>();
        let exact = queries
            .iter()
            .map(|query| exact_ids(&vectors, query))
            .collect::<Vec<_>>();
        let filtered_candidates = query_seeds
            .iter()
            .copied()
            .map(|seed| filtered_candidate_set(&ordinal_to_node, seed))
            .collect::<Vec<_>>();
        Self {
            dimension,
            graph,
            label,
            property,
            queries,
            exact,
            node_to_ordinal,
            filtered_candidates,
        }
    }

    fn rows(&self) -> usize {
        self.node_to_ordinal.len()
    }

    fn query_count(&self) -> usize {
        self.queries.len()
    }

    fn filtered_candidate_count(&self) -> usize {
        self.filtered_candidates
            .first()
            .map_or(0, VectorCandidateSet::len)
    }

    fn total_overlap(&self) -> usize {
        self.queries
            .iter()
            .zip(&self.exact)
            .map(|(query, exact)| {
                let approx = self.production_ids(query);
                exact.iter().filter(|id| approx.contains(id)).count()
            })
            .sum()
    }

    fn recall_basis_points(&self) -> usize {
        self.total_overlap() * 10_000 / (self.queries.len() * K)
    }

    fn batch_recall_basis_points(&self) -> usize {
        self.total_batch_overlap() * 10_000 / (self.queries.len() * K)
    }

    fn filtered_recall_basis_points(&self) -> usize {
        self.filtered_total_overlap() * 10_000 / (self.queries.len() * K)
    }

    fn filtered_batch_recall_basis_points(&self) -> usize {
        self.filtered_batch_total_overlap() * 10_000 / (self.queries.len() * K)
    }

    fn memory_suffix(&self) -> String {
        let usage = self
            .graph
            .read()
            .vector_index_for(&self.label, &self.property)
            .expect("bench fixture has TurboQuant index")
            .memory_usage();
        memory_suffix(
            usage.turbo_quant_index_bytes,
            self.rows() * self.dimension * size_of::<f32>(),
        )
    }

    fn production_ids(&self, query: &VectorValue) -> Vec<usize> {
        self.graph
            .read()
            .approximate_vector_search_nodes_checked(
                &self.label,
                &self.property,
                query,
                ApproximateVectorSearchOptions::new(
                    VectorMetric::Cosine,
                    K,
                    PRODUCTION_SEARCH_WIDTH,
                ),
                CancellationChecker::disabled(),
            )
            .expect("production TurboQuant search succeeds")
            .into_iter()
            .map(|hit| {
                *self
                    .node_to_ordinal
                    .get(&hit.node_id)
                    .expect("search hit node was inserted by this fixture")
            })
            .collect()
    }

    fn total_batch_overlap(&self) -> usize {
        self.production_batch_ids()
            .iter()
            .zip(&self.exact)
            .map(|(approx, exact)| exact.iter().filter(|id| approx.contains(id)).count())
            .sum()
    }

    fn filtered_total_overlap(&self) -> usize {
        self.filtered_production_ids()
            .iter()
            .zip(&self.exact)
            .map(|(approx, exact)| exact.iter().filter(|id| approx.contains(id)).count())
            .sum()
    }

    fn filtered_batch_total_overlap(&self) -> usize {
        self.filtered_batch_production_ids()
            .iter()
            .zip(&self.exact)
            .map(|(approx, exact)| exact.iter().filter(|id| approx.contains(id)).count())
            .sum()
    }

    fn production_batch_ids(&self) -> Vec<Vec<usize>> {
        self.graph
            .read()
            .approximate_vector_search_nodes_batch_checked(
                &self.label,
                &self.property,
                &self.queries,
                ApproximateVectorSearchOptions::new(
                    VectorMetric::Cosine,
                    K,
                    PRODUCTION_SEARCH_WIDTH,
                ),
                CancellationChecker::disabled(),
            )
            .expect("production TurboQuant batch search succeeds")
            .into_iter()
            .map(|hits| {
                hits.into_iter()
                    .map(|hit| {
                        *self
                            .node_to_ordinal
                            .get(&hit.node_id)
                            .expect("search hit node was inserted by this fixture")
                    })
                    .collect()
            })
            .collect()
    }

    fn filtered_production_ids(&self) -> Vec<Vec<usize>> {
        self.queries
            .iter()
            .zip(&self.filtered_candidates)
            .map(|(query, candidates)| {
                self.graph
                    .approximate_vector_search_candidate_set_checked(
                        &self.label,
                        &self.property,
                        query,
                        candidates,
                        ApproximateVectorSearchOptions::new(
                            VectorMetric::Cosine,
                            K,
                            PRODUCTION_SEARCH_WIDTH,
                        ),
                        CancellationChecker::disabled(),
                    )
                    .expect("production filtered TurboQuant search succeeds")
                    .into_iter()
                    .map(|hit| {
                        *self
                            .node_to_ordinal
                            .get(&hit.node_id)
                            .expect("search hit node was inserted by this fixture")
                    })
                    .collect()
            })
            .collect()
    }

    fn filtered_batch_production_ids(&self) -> Vec<Vec<usize>> {
        self.graph
            .approximate_vector_search_candidate_sets_batch_checked(
                &self.label,
                &self.property,
                &self.queries,
                &self.filtered_candidates,
                ApproximateVectorSearchOptions::new(
                    VectorMetric::Cosine,
                    K,
                    PRODUCTION_SEARCH_WIDTH,
                ),
                CancellationChecker::disabled(),
            )
            .expect("production filtered TurboQuant batch search succeeds")
            .into_iter()
            .map(|hits| {
                hits.into_iter()
                    .map(|hit| {
                        *self
                            .node_to_ordinal
                            .get(&hit.node_id)
                            .expect("search hit node was inserted by this fixture")
                    })
                    .collect()
            })
            .collect()
    }
}

impl DimensionFixture {
    fn build(dimension: usize, variant: TurboQuantVariant) -> Self {
        let vectors = (0..ROWS)
            .map(|seed| dimension_vector(seed, dimension, 0.0))
            .collect::<Vec<_>>();
        let queries = (0..QUERY_COUNT)
            .map(|idx| {
                let cluster = idx % cluster_count();
                let seed = cluster + (ROWS / cluster_count() / 2) * cluster_count();
                dimension_vector(seed.min(ROWS - 1), dimension, 0.0003)
            })
            .collect::<Vec<_>>();
        let exact = queries
            .iter()
            .map(|query| exact_ids(&vectors, query))
            .collect::<Vec<_>>();
        let turbo = TurboQuantIndex::build(&vectors, variant);
        Self {
            dimension,
            vectors,
            queries,
            exact,
            turbo,
        }
    }

    fn rows(&self) -> usize {
        self.vectors.len()
    }

    fn query_count(&self) -> usize {
        self.queries.len()
    }

    fn total_overlap(&self) -> usize {
        self.queries
            .iter()
            .zip(&self.exact)
            .map(|(query, exact)| {
                let approx = self.turbo.search_all(&self.vectors, query, K);
                exact.iter().filter(|id| approx.contains(id)).count()
            })
            .sum()
    }

    fn recall_basis_points(&self) -> usize {
        self.total_overlap() * 10_000 / (self.queries.len() * K)
    }

    fn memory_suffix(&self) -> String {
        memory_suffix(
            self.turbo.estimated_bytes(),
            self.vectors.len() * self.dimension * size_of::<f32>(),
        )
    }
}

fn exact_ids(vectors: &[VectorValue], query: &VectorValue) -> Vec<usize> {
    exact_vector_top_k(VectorMetric::Cosine, query, vectors.iter().enumerate(), K)
        .expect("dimension-projection benchmark vectors have matching dimensions")
        .into_iter()
        .map(|hit| hit.key)
        .collect()
}

fn dimension_vector(seed: usize, dimension: usize, jitter: f32) -> VectorValue {
    let cluster = seed % cluster_count();
    let ordinal = seed / cluster_count();
    let components = (0..dimension)
        .map(|dim| {
            let base = ((((cluster + 1) * (dim + 3)) % 97) as f32 - 48.0) / 48.0;
            let local = ((((ordinal + 5) * (dim + 11)) % 31) as f32 - 15.0) * 0.001;
            base + local + jitter
        })
        .collect::<Vec<_>>();
    VectorValue::new(components).expect("benchmark vector is finite and non-empty")
}

fn filtered_candidate_set(nodes: &[NodeId], seed: usize) -> VectorCandidateSet {
    let half_window = FILTERED_CANDIDATE_LEN / 2;
    let start = seed
        .saturating_sub(half_window)
        .min(nodes.len().saturating_sub(FILTERED_CANDIDATE_LEN));
    let end = start + FILTERED_CANDIDATE_LEN;
    let cluster = seed % cluster_count();
    let mut candidates = nodes[start..end].to_vec();
    candidates.extend(
        (cluster..nodes.len())
            .step_by(cluster_count())
            .map(|row| nodes[row]),
    );
    VectorCandidateSet::from_nodes(candidates)
}

fn cluster_count() -> usize {
    40
}

fn compact_count(value: usize) -> String {
    if value >= 1_000 && value.is_multiple_of(1_000) {
        format!("{}k", value / 1_000)
    } else {
        value.to_string()
    }
}

fn memory_suffix(compressed_bytes: usize, full_bytes: usize) -> String {
    format!("m{}-full{}", compressed_bytes / 1024, full_bytes / 1024)
}

criterion_group! {
    name = vector_turbo_projection;
    config = common::criterion_config();
    targets = bench_turbo_quant_dimension_projection,
        bench_blocked_turbo_quant_dimension_projection,
        bench_blocked_wide_turbo_quant_dimension_projection,
        bench_blocked_fast_scan_turbo_quant_dimension_projection,
        bench_production_turbo_quant_dimension_projection,
        bench_production_turbo_quant_batch_dimension_projection,
        bench_production_turbo_quant_filtered_dimension_projection,
        bench_production_turbo_quant_filtered_batch_dimension_projection
}
criterion_main!(vector_turbo_projection);
