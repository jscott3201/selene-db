#![allow(missing_docs)]
//! Criterion benches for plan lowering, optimization, and IR cloning.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use selene_gql::{EmptyProcedureRegistry, ImplDefinedCaps, analyze, optimize, parse, plan};
use selene_testing::PlanCorpus;

fn bench_plan_optimize_corpus(c: &mut Criterion) {
    let entries = common::corpus_entries();
    let empty = EmptyProcedureRegistry;
    let mock = PlanCorpus::standard_mock_registry();
    let catalog = PlanCorpus::standard_mock_catalog();
    let mut group = c.benchmark_group("gql_plan_optimize_corpus");
    group.throughput(Throughput::Elements(entries.len() as u64));
    group.bench_function("m5c", |b| {
        b.iter_batched(
            || {
                entries
                    .iter()
                    .map(|entry| {
                        let registry = common::registry_for(entry, &empty, &mock);
                        let statement = parse(entry.source).expect("source parses");
                        let analyzed = analyze(statement, registry, None).expect("source analyzes");
                        (entry.clone(), analyzed)
                    })
                    .collect::<Vec<_>>()
            },
            |prepared| {
                for (entry, analyzed) in prepared {
                    let registry = common::registry_for(&entry, &empty, &mock);
                    let planned = plan(&analyzed, registry).expect("source plans");
                    let caps = ImplDefinedCaps::default();
                    let ctx = common::context_for(&entry, &caps, &catalog);
                    let optimized = optimize(planned, &ctx);
                    std::hint::black_box(optimized.pipeline.len());
                }
            },
            criterion::BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn bench_plan_ir_clone(c: &mut Criterion) {
    let plan = common::representative_plan();
    let mut group = c.benchmark_group("gql_plan_ir_clone");
    group.throughput(Throughput::Elements(1));
    group.bench_function("representative", |b| {
        b.iter(|| {
            let cloned = plan.clone();
            std::hint::black_box(cloned.pipeline.len());
        });
    });
    group.finish();
}

criterion_group! {
    name = plan_optimize_group;
    config = common::criterion_config();
    targets = bench_plan_optimize_corpus, bench_plan_ir_clone
}
criterion_main!(plan_optimize_group);
