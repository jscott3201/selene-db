#![allow(missing_docs)]
//! Criterion benches for single-snapshot graph read hot paths.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, VectorMetric, VectorValue, intern};
use selene_graph::{SeleneGraph, SharedGraph};
use selene_testing::BenchProfile;

fn bench_node_fetch(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_node_fetch");
    for fixture in common::fixtures() {
        group.throughput(Throughput::Elements(fixture.scale() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let props = fixture.graph().node_properties(fixture.sample_node_id());
                    std::hint::black_box(props.map(selene_core::PropertyMap::len));
                });
            },
        );
    }
    group.finish();
}

fn bench_label_index(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_label_index_lookup");
    for fixture in common::fixtures() {
        group.bench_with_input(
            BenchmarkId::from_parameter(fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let rows = fixture.graph().nodes_with_label(&fixture.sensor_label());
                    std::hint::black_box(rows.map(roaring::RoaringBitmap::len));
                });
            },
        );
    }
    group.finish();
}

fn bench_typed_index_point(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_typed_index_point");
    for fixture in common::fixtures() {
        group.bench_with_input(
            BenchmarkId::from_parameter(fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let rows = fixture.graph().nodes_with_property_eq(
                        &fixture.person_label(),
                        &fixture.age_key(),
                        &Value::Int(fixture.sample_age_value()),
                    );
                    std::hint::black_box(rows.map(|rows| rows.len()));
                });
            },
        );
    }
    group.finish();
}

fn bench_typed_index_range(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_typed_index_range");
    for fixture in common::fixtures() {
        group.bench_with_input(
            BenchmarkId::from_parameter(fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let start = Value::Int(fixture.sample_age_value());
                    let end = Value::Int(fixture.sample_age_value() + 20);
                    let rows = fixture.graph().nodes_with_property_range(
                        &fixture.person_label(),
                        &fixture.age_key(),
                        start..end,
                    );
                    std::hint::black_box(rows.map(|rows| rows.len()));
                });
            },
        );
    }
    group.finish();
}

fn bench_composite_index_proxy(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_composite_index_proxy");
    for fixture in common::fixtures() {
        group.bench_with_input(
            BenchmarkId::from_parameter(fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let age_rows = fixture
                        .graph()
                        .nodes_with_property_eq(
                            &fixture.person_label(),
                            &fixture.age_key(),
                            &Value::Int(fixture.sample_age_value()),
                        )
                        .map(std::borrow::Cow::into_owned)
                        .unwrap_or_default();
                    let mut name_rows = fixture
                        .graph()
                        .nodes_with_property_eq(
                            &fixture.person_label(),
                            &fixture.name_key(),
                            &Value::String(fixture.sample_name_value()),
                        )
                        .map(std::borrow::Cow::into_owned)
                        .unwrap_or_default();
                    name_rows &= age_rows;
                    std::hint::black_box(name_rows.len());
                });
            },
        );
    }
    group.finish();
}

fn bench_exact_vector_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_exact_vector_scan");
    for scale in BenchProfile::from_env().scales() {
        let fixture = VectorFixture::build(*scale, 128);
        group.throughput(Throughput::Elements(fixture.scale() as u64));
        group.bench_with_input(
            BenchmarkId::new("squared_euclidean_dim128_k10", fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let hits = fixture
                        .graph()
                        .exact_vector_search_nodes(
                            &fixture.label(),
                            &fixture.embedding_key(),
                            fixture.query(),
                            VectorMetric::SquaredEuclidean,
                            10,
                        )
                        .expect("fixture vectors have matching dimensions");
                    std::hint::black_box(hits.len());
                });
            },
        );
    }
    group.finish();
}

#[derive(Clone, Debug)]
struct VectorFixture {
    scale: usize,
    graph: SeleneGraph,
    label: IStr,
    embedding_key: IStr,
    query: VectorValue,
}

impl VectorFixture {
    fn build(scale: usize, dimension: usize) -> Self {
        let scale = scale.max(1);
        let label = intern("VectorDoc").expect("bench label is valid");
        let embedding_key = intern("embedding").expect("bench key is valid");
        let shared = SharedGraph::new(GraphId::new(9_000 + scale as u64));
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            for idx in 0..scale {
                let vector = Value::Vector(vector_value(idx, dimension));
                let props = PropertyMap::from_pairs([(embedding_key.clone(), vector)])
                    .expect("bench vector properties are valid");
                mutator
                    .create_node(LabelSet::single(label.clone()), props)
                    .expect("bench vector node insert succeeds");
            }
            txn.commit().expect("bench vector fixture commit succeeds");
        }
        Self {
            scale,
            graph: shared.read().as_ref().clone(),
            label,
            embedding_key,
            query: vector_value(0, dimension),
        }
    }

    const fn graph(&self) -> &SeleneGraph {
        &self.graph
    }

    const fn scale(&self) -> usize {
        self.scale
    }

    fn label(&self) -> IStr {
        self.label.clone()
    }

    fn embedding_key(&self) -> IStr {
        self.embedding_key.clone()
    }

    const fn query(&self) -> &VectorValue {
        &self.query
    }
}

fn vector_value(seed: usize, dimension: usize) -> VectorValue {
    let components: Vec<f32> = (0..dimension)
        .map(|dim| {
            let raw = (seed.wrapping_mul(31) + dim.wrapping_mul(17)) % 1_000;
            raw as f32 / 1_000.0
        })
        .collect();
    VectorValue::new(components).expect("bench vector is valid")
}

criterion_group! {
    name = graph_reads;
    config = common::criterion_config();
    targets = bench_node_fetch, bench_label_index, bench_typed_index_point,
        bench_typed_index_range, bench_composite_index_proxy, bench_exact_vector_scan
}
criterion_main!(graph_reads);
