#![allow(missing_docs)]
#![allow(dead_code)]
//! Criterion benches for TurboQuant storage/search across embedding dimensions.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;
#[path = "vector_pq_support/turbo_quant.rs"]
mod turbo_quant;

use std::mem::size_of;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{VectorMetric, VectorValue, exact_vector_top_k};
use turbo_quant::{
    TurboQuantCalibration, TurboQuantCodebook, TurboQuantIndex, TurboQuantScorer, TurboQuantVariant,
};

const ROWS: usize = 10_000;
const QUERY_COUNT: usize = 8;
const K: usize = 10;
const DIMENSIONS: [usize; 3] = [128, 768, 1536];
const VARIANT: TurboQuantVariant = TurboQuantVariant {
    name: "tqplus4lut_c1024",
    bit_width: 4,
    candidates: 1024,
    codebook: TurboQuantCodebook::NormalLloydMax,
    calibration: TurboQuantCalibration::Quantile,
    scorer: TurboQuantScorer::ByteLut,
};

fn bench_turbo_quant_dimension_projection(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_turbo_quant_dimension_projection");
    for dimension in DIMENSIONS {
        let fixture = DimensionFixture::build(dimension);
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

#[derive(Debug)]
struct DimensionFixture {
    dimension: usize,
    vectors: Vec<VectorValue>,
    queries: Vec<VectorValue>,
    exact: Vec<Vec<usize>>,
    turbo: TurboQuantIndex,
}

impl DimensionFixture {
    fn build(dimension: usize) -> Self {
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
        let turbo = TurboQuantIndex::build(&vectors, VARIANT);
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
    targets = bench_turbo_quant_dimension_projection
}
criterion_main!(vector_turbo_projection);
