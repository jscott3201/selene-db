#![allow(missing_docs)]
//! CORE-06 gating bench: `Value` clone cost (dominated by the enum size).
//!
//! `Value` currently inlines `jiff::Span` (`Duration`) and two `jiff::Zoned`
//! variants (`ZonedDateTime`/`ZonedTime`), so `size_of::<Value>` is large and
//! EVERY `Value` / `PropertyMap` clone memcpys that many bytes regardless of the
//! active variant. This bench measures the clone cost; the companion
//! compile-time `size_of::<Value>` ceiling in `value.rs` is the zero-cost
//! re-bloat tripwire. Boxing the large time variants (CORE-06) should shrink the
//! size and speed these rows — lower the ceiling when it lands.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{PropertyMap, Value, intern};

const N: usize = 1_024;

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
        .map(|i| match i % 5 {
            0 => Value::Int(i as i64),
            1 => Value::Float(i as f64 * 1.5),
            2 => Value::String(intern(&format!("v{}", i % 64)).expect("string interns")),
            3 => Value::Duration(Box::new(span)),
            _ => Value::ZonedDateTime(Box::new(zoned.clone())),
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

criterion_group! {
    name = value_clone;
    config = bench_config();
    targets = bench_value_clone
}
criterion_main!(value_clone);
