#![allow(missing_docs)]
//! Criterion benches for single-snapshot graph read hot paths.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;
#[path = "single_graph/edge.rs"]
mod edge;
#[path = "single_graph/json.rs"]
mod json;
mod single_graph_ann_recall;
mod single_graph_candidate_set;
mod single_graph_vector_batch;
#[path = "single_graph/vector.rs"]
mod vector;

use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{
    CancellationChecker, DbString, GraphId, JsonPathSelector, JsonValue, LabelSet, PropertyMap,
    Value, VectorMetric, VectorValue, db_string,
};
use selene_graph::{SeleneGraph, SharedGraph, VectorIndexKind, VectorIndexMemoryUsage};
use selene_testing::BenchProfile;
use single_graph_ann_recall::{ANN_RECALL_PROFILES, AnnRecallFixture};

const ANN_RECALL_K: usize = 10;
const ANN_RECALL_QUERIES: usize = 16;

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
    vector::bench_exact_vector_scan(c);
}

fn bench_exact_json_contains_scan(c: &mut Criterion) {
    json::bench_exact_json_contains_scan(c);
}

fn bench_exact_json_path_exists_scan(c: &mut Criterion) {
    json::bench_exact_json_path_exists_scan(c);
}

fn bench_exact_json_path_contains_scan(c: &mut Criterion) {
    json::bench_exact_json_path_contains_scan(c);
}

fn bench_exact_json_path_value_scan(c: &mut Criterion) {
    json::bench_exact_json_path_value_scan(c);
}

fn bench_ann_recall(c: &mut Criterion) {
    vector::bench_ann_recall(c);
}

fn vector_scan_scales() -> Vec<usize> {
    vector::vector_scan_scales()
}

fn vector_value(seed: usize, dimension: usize) -> VectorValue {
    vector::vector_value(seed, dimension)
}

fn deadline_checker() -> CancellationChecker<'static> {
    CancellationChecker::new(None, Some(Instant::now() + Duration::from_secs(3600)))
}

criterion_group! {
    name = graph_reads;
    config = common::criterion_config();
    targets = bench_node_fetch, bench_label_index, bench_typed_index_point,
        bench_typed_index_range, bench_composite_index_proxy,
        edge::bench_edge_property_scan, edge::bench_edge_property_index_lookup,
        edge::bench_point_connected_traversal,
        bench_exact_vector_scan,
        single_graph_vector_batch::bench_exact_vector_batch_scan,
        bench_exact_json_contains_scan, bench_exact_json_path_exists_scan,
        bench_exact_json_path_contains_scan, bench_exact_json_path_value_scan,
        single_graph_candidate_set::bench_vector_candidate_set, bench_ann_recall
}
criterion_main!(graph_reads);
