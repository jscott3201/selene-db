#![allow(missing_docs)]
//! Criterion benches for plan-corpus parsing throughput.

mod common;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};

fn bench_parse_corpus(c: &mut Criterion) {
    let entries = common::corpus_entries();
    let bytes = entries
        .iter()
        .map(|entry| entry.source.len())
        .sum::<usize>();
    let mut group = c.benchmark_group("gql_parse_corpus");
    group.throughput(Throughput::Bytes(bytes as u64));
    group.bench_function("m5c", |b| {
        b.iter(|| {
            for entry in &entries {
                std::hint::black_box(selene_gql::parse(entry.source).expect("source parses"));
            }
        });
    });
    group.finish();
}

criterion_group! {
    name = parse_group;
    config = common::criterion_config();
    targets = bench_parse_corpus
}
criterion_main!(parse_group);
