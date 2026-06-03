#![allow(missing_docs)]
//! CORE-06 gating bench: `Value` clone cost (dominated by the enum size) plus
//! native vector-value construction and serde baselines.
//!
//! `Value` currently inlines `jiff::Span` (`Duration`) and two `jiff::Zoned`
//! variants (`ZonedDateTime`/`ZonedTime`), so `size_of::<Value>` is large and
//! EVERY `Value` / `PropertyMap` clone memcpys that many bytes regardless of the
//! active variant. This bench measures the clone cost; the companion
//! compile-time `size_of::<Value>` ceiling in `value.rs` is the zero-cost
//! re-bloat tripwire. Boxing the large time variants (CORE-06) should shrink the
//! size and speed these rows — lower the ceiling when it lands.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{
    PropertyMap, Value, VectorMetric, VectorTopK, VectorValue, exact_vector_top_k, intern,
};

const N: usize = 1_024;
const VECTOR_DIMS: &[usize] = &[128, 768, 1536];
const EXACT_TOP_K_CANDIDATES: usize = 2_048;
const EXACT_TOP_K: usize = 10;

// Profile-aware criterion config WITHOUT a selene-testing dep (that crate
// depends on selene-core, so importing BenchProfile here would cycle).
fn bench_config() -> Criterion {
    let (samples, ms) = match std::env::var("SELENE_BENCH_PROFILE").ok().as_deref() {
        Some("full") | Some("stress") => (30usize, 1_500u64),
        _ => (10, 500),
    };
    Criterion::default()
        .sample_size(samples)
        .warm_up_time(std::time::Duration::from_millis(100))
        .measurement_time(std::time::Duration::from_millis(ms))
}

fn span() -> jiff::Span {
    "PT1H30M".parse().expect("fixed span parses")
}

fn zoned() -> jiff::Zoned {
    "2024-06-01T12:00:00[UTC]"
        .parse()
        .expect("fixed zoned parses")
}

fn mixed_values() -> Vec<Value> {
    let span = span();
    let zoned = zoned();
    (0..N)
        .map(|i| match i % 6 {
            0 => Value::Int(i as i64),
            1 => Value::Float(i as f64 * 1.5),
            2 => Value::String(intern(&format!("v{}", i % 64)).expect("string interns")),
            3 => Value::Duration(Box::new(span)),
            4 => Value::ZonedDateTime(Box::new(zoned.clone())),
            _ => Value::Vector(VectorValue::new(vector_components(32)).expect("vector is valid")),
        })
        .collect()
}

fn mixed_property_map() -> PropertyMap {
    PropertyMap::from_pairs([
        (intern("i").expect("key"), Value::Int(42)),
        (intern("f").expect("key"), Value::Float(2.5)),
        (
            intern("s").expect("key"),
            Value::String(intern("hello").expect("value")),
        ),
        (intern("d").expect("key"), Value::Duration(Box::new(span()))),
        (
            intern("z").expect("key"),
            Value::ZonedDateTime(Box::new(zoned())),
        ),
    ])
    .expect("property map fits core caps")
}

fn wide_property_pairs(width: usize) -> Vec<(selene_core::IStr, Value)> {
    (0..width)
        .rev()
        .map(|idx| {
            (
                intern(&format!("wide_property_{idx:04}")).expect("key interns"),
                Value::Int(idx as i64),
            )
        })
        .collect()
}

// `print_stderr` deny is locally relaxed to surface the current `Value` size
// in bench output (so the CORE-06 shrink is visible run-to-run).
#[allow(clippy::print_stderr)]
fn bench_value_clone(c: &mut Criterion) {
    eprintln!(
        "[core_value_clone] size_of::<Value>() = {} bytes",
        std::mem::size_of::<Value>()
    );
    let mut group = c.benchmark_group("core_value_clone");

    let values = mixed_values();
    group.throughput(Throughput::Elements(N as u64));
    group.bench_function("vec_mixed_1024", |b| {
        b.iter(|| black_box(black_box(&values).clone()));
    });

    let map = mixed_property_map();
    group.bench_function("property_map_5", |b| {
        b.iter(|| black_box(black_box(&map).clone()));
    });

    let pairs = wide_property_pairs(256);
    group.throughput(Throughput::Elements(pairs.len() as u64));
    group.bench_function("property_map_from_pairs_256_reverse", |b| {
        b.iter(|| {
            PropertyMap::from_pairs(black_box(pairs.iter().cloned()))
                .expect("property map fits core caps")
        });
    });

    group.finish();
}

fn vector_components(dim: usize) -> Vec<f32> {
    vector_components_seeded(dim, 0)
}

fn vector_components_seeded(dim: usize, seed: usize) -> Vec<f32> {
    (0..dim)
        .map(|idx| (((idx * 31 + seed * 17) % 1_021) as f32 - 510.0) / 256.0)
        .collect()
}

fn bench_vector_value(c: &mut Criterion) {
    let mut group = c.benchmark_group("core_vector_value");
    for &dim in VECTOR_DIMS {
        group.throughput(Throughput::Elements(dim as u64));
        group.bench_with_input(
            BenchmarkId::new("construct_validate", dim),
            &dim,
            |b, &dim| {
                b.iter_batched(
                    || vector_components(dim),
                    |components| VectorValue::new(black_box(components)).expect("vector is valid"),
                    BatchSize::SmallInput,
                );
            },
        );

        let vector = VectorValue::new(vector_components(dim)).expect("vector is valid");
        group.bench_with_input(BenchmarkId::new("clone_arc", dim), &vector, |b, vector| {
            b.iter(|| black_box(black_box(vector).clone()));
        });

        let value = Value::Vector(vector);
        group.bench_with_input(
            BenchmarkId::new("postcard_roundtrip", dim),
            &value,
            |b, value| {
                b.iter_batched(
                    || value.clone(),
                    |value| {
                        let bytes = postcard::to_allocvec(&value).expect("vector serializes");
                        let decoded: Value =
                            postcard::from_bytes(&bytes).expect("vector deserializes");
                        black_box(decoded)
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_vector_distance(c: &mut Criterion) {
    let mut group = c.benchmark_group("core_vector_distance");
    for &(name, metric) in &[
        ("squared_euclidean", VectorMetric::SquaredEuclidean),
        ("cosine", VectorMetric::Cosine),
        ("negative_inner_product", VectorMetric::NegativeInnerProduct),
    ] {
        for &dim in VECTOR_DIMS {
            group.throughput(Throughput::Elements(dim as u64));
            let lhs = VectorValue::new(vector_components_seeded(dim, 1)).expect("vector is valid");
            let rhs = VectorValue::new(vector_components_seeded(dim, 2)).expect("vector is valid");
            group.bench_with_input(BenchmarkId::new(name, dim), &dim, |b, _| {
                b.iter(|| {
                    metric
                        .distance(black_box(&lhs), black_box(&rhs))
                        .expect("same dim")
                });
            });
        }
    }
    group.finish();
}

fn bench_vector_exact_top_k(c: &mut Criterion) {
    let mut group = c.benchmark_group("core_vector_exact_top_k");
    group.throughput(Throughput::Elements(EXACT_TOP_K_CANDIDATES as u64));

    let query = VectorValue::new(vector_components_seeded(128, 0)).expect("query is valid");
    let candidates: Vec<VectorValue> = (0..EXACT_TOP_K_CANDIDATES)
        .map(|idx| VectorValue::new(vector_components_seeded(128, idx + 1)).expect("valid vector"))
        .collect();
    let candidate_refs: Vec<(usize, &VectorValue)> = candidates.iter().enumerate().collect();

    for &(name, metric) in &[
        (
            "squared_euclidean_2048x128_k10",
            VectorMetric::SquaredEuclidean,
        ),
        ("cosine_2048x128_k10", VectorMetric::Cosine),
    ] {
        group.bench_function(name, |b| {
            b.iter(|| {
                exact_vector_top_k(
                    metric,
                    black_box(&query),
                    candidate_refs
                        .iter()
                        .map(|(key, vector)| (black_box(*key), black_box(*vector))),
                    black_box(EXACT_TOP_K),
                )
                .expect("all dimensions match")
            });
        });
    }

    group.bench_function("cosine_unbound_loop_2048x128_k10", |b| {
        b.iter(|| {
            let mut top_k = VectorTopK::new(black_box(EXACT_TOP_K));
            for (key, vector) in &candidate_refs {
                let distance = VectorMetric::Cosine
                    .distance(black_box(&query), black_box(*vector))
                    .expect("all dimensions match");
                top_k.push_distance(black_box(*key), distance);
            }
            top_k.into_hits()
        });
    });

    group.finish();
}

criterion_group! {
    name = value_clone;
    config = bench_config();
    targets = bench_value_clone, bench_vector_value, bench_vector_distance, bench_vector_exact_top_k
}
criterion_main!(value_clone);
