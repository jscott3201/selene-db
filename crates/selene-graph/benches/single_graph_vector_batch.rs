use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::{
    CancellationChecker, DbString, GraphId, LabelSet, PropertyMap, Value, VectorMetric,
    VectorValue, db_string,
};
use selene_graph::{SeleneGraph, SharedGraph, VectorIndexKind};
use selene_testing::BenchProfile;

const BATCH_QUERY_COUNT: usize = 8;
const VECTOR_DIMENSION: usize = 128;
const VECTOR_K: usize = 10;

pub(crate) fn bench_exact_vector_batch_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_exact_vector_batch_scan");
    for scale in vector_scan_scales() {
        for &(index_name, index_kind) in &[
            ("unindexed", None),
            ("flat_index", Some(VectorIndexKind::Flat)),
        ] {
            let fixture = BatchVectorFixture::build(scale, index_kind);
            group.throughput(Throughput::Elements(
                (fixture.scale() * BATCH_QUERY_COUNT) as u64,
            ));
            group.bench_with_input(
                BenchmarkId::new(
                    format!("{index_name}_squared_euclidean_q8_dim128_k10"),
                    fixture.scale(),
                ),
                &fixture,
                |b, fixture| {
                    b.iter(|| {
                        let hits = fixture
                            .graph()
                            .exact_vector_search_nodes_batch_checked(
                                fixture.label(),
                                fixture.embedding_key(),
                                fixture.queries(),
                                VectorMetric::SquaredEuclidean,
                                VECTOR_K,
                                CancellationChecker::disabled(),
                            )
                            .expect("fixture vectors have matching dimensions");
                        std::hint::black_box(hits.iter().map(Vec::len).sum::<usize>());
                    });
                },
            );
            group.bench_with_input(
                BenchmarkId::new(
                    format!("{index_name}_cosine_q8_dim128_k10"),
                    fixture.scale(),
                ),
                &fixture,
                |b, fixture| {
                    b.iter(|| {
                        let hits = fixture
                            .graph()
                            .exact_vector_search_nodes_batch_checked(
                                fixture.label(),
                                fixture.embedding_key(),
                                fixture.queries(),
                                VectorMetric::Cosine,
                                VECTOR_K,
                                CancellationChecker::disabled(),
                            )
                            .expect("fixture vectors have matching dimensions");
                        std::hint::black_box(hits.iter().map(Vec::len).sum::<usize>());
                    });
                },
            );
            group.bench_with_input(
                BenchmarkId::new(
                    format!("{index_name}_squared_euclidean_q8_dim128_k10_checked_with_deadline"),
                    fixture.scale(),
                ),
                &fixture,
                |b, fixture| {
                    let checker = deadline_checker();
                    b.iter(|| {
                        let hits = fixture
                            .graph()
                            .exact_vector_search_nodes_batch_checked(
                                fixture.label(),
                                fixture.embedding_key(),
                                fixture.queries(),
                                VectorMetric::SquaredEuclidean,
                                VECTOR_K,
                                checker,
                            )
                            .expect("fixture vectors have matching dimensions");
                        std::hint::black_box(hits.iter().map(Vec::len).sum::<usize>());
                    });
                },
            );
        }
    }
    group.finish();
}

#[derive(Clone, Debug)]
struct BatchVectorFixture {
    scale: usize,
    graph: SeleneGraph,
    label: DbString,
    embedding_key: DbString,
    queries: Vec<VectorValue>,
}

impl BatchVectorFixture {
    fn build(scale: usize, index_kind: Option<VectorIndexKind>) -> Self {
        let scale = scale.max(1);
        let label = db_string("VectorDocBatch").expect("bench label is valid");
        let embedding_key = db_string("embedding").expect("bench key is valid");
        let shared = SharedGraph::new(GraphId::new(9_100 + scale as u64));
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            for idx in 0..scale {
                let props = PropertyMap::from_pairs([(
                    embedding_key.clone(),
                    Value::Vector(vector_value(idx, VECTOR_DIMENSION)),
                )])
                .expect("bench vector properties are valid");
                mutator
                    .create_node(LabelSet::single(label.clone()), props)
                    .expect("bench vector node insert succeeds");
            }
            if let Some(kind) = index_kind {
                mutator
                    .create_vector_index(
                        label.clone(),
                        embedding_key.clone(),
                        kind,
                        VECTOR_DIMENSION
                            .try_into()
                            .expect("bench vector dimension fits u32"),
                    )
                    .expect("bench vector index build succeeds");
            }
            txn.commit().expect("bench vector fixture commit succeeds");
        }
        Self {
            scale,
            graph: shared.read().as_ref().clone(),
            label,
            embedding_key,
            queries: (0..BATCH_QUERY_COUNT)
                .map(|seed| vector_value(seed, VECTOR_DIMENSION))
                .collect(),
        }
    }

    const fn graph(&self) -> &SeleneGraph {
        &self.graph
    }

    const fn scale(&self) -> usize {
        self.scale
    }

    const fn label(&self) -> &DbString {
        &self.label
    }

    const fn embedding_key(&self) -> &DbString {
        &self.embedding_key
    }

    fn queries(&self) -> &[VectorValue] {
        &self.queries
    }
}

fn vector_scan_scales() -> Vec<usize> {
    std::env::var("SELENE_VECTOR_BENCH_SCALES")
        .ok()
        .and_then(|raw| {
            let mut scales: Vec<_> = raw
                .split(',')
                .filter_map(|part| part.trim().parse::<usize>().ok())
                .filter(|scale| *scale > 0)
                .collect();
            scales.sort_unstable();
            scales.dedup();
            (!scales.is_empty()).then_some(scales)
        })
        .unwrap_or_else(|| BenchProfile::from_env().scales().to_vec())
}

fn deadline_checker() -> CancellationChecker<'static> {
    CancellationChecker::new(None, Some(Instant::now() + Duration::from_secs(3600)))
}

fn vector_value(seed: usize, dimension: usize) -> VectorValue {
    VectorValue::new(vector_components(seed, dimension)).expect("bench vector is valid")
}

fn vector_components(seed: usize, dimension: usize) -> Vec<f32> {
    (0..dimension)
        .map(|dim| {
            let raw = (seed.wrapping_mul(31) + dim.wrapping_mul(17)) % 1_000;
            raw as f32 / 1_000.0
        })
        .collect()
}
