#![allow(missing_docs)]
//! iai-callgrind gates for plan-only GQL compilation hot paths.

mod common;

use iai_callgrind::{
    Callgrind, EventKind, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main,
};
use selene_gql::{EmptyProcedureRegistry, analyze, optimize, parse, plan};
use selene_testing::PlanCorpus;

#[library_benchmark]
fn parse_corpus() -> usize {
    let entries = common::corpus_entries();
    std::hint::black_box(
        entries
            .iter()
            .map(|entry| parse(entry.source).expect("source parses").span().end() as usize)
            .sum(),
    )
}

#[library_benchmark]
fn analyze_corpus() -> usize {
    let entries = common::corpus_entries();
    let empty = EmptyProcedureRegistry;
    let mock = PlanCorpus::standard_mock_registry();
    std::hint::black_box(
        entries
            .iter()
            .map(|entry| {
                let registry = common::registry_for(entry, &empty, &mock);
                let statement = parse(entry.source).expect("source parses");
                analyze(statement, registry, None)
                    .expect("source analyzes")
                    .expr_types
                    .len()
            })
            .sum(),
    )
}

#[library_benchmark]
fn plan_optimize_corpus() -> usize {
    let entries = common::corpus_entries();
    let empty = EmptyProcedureRegistry;
    let mock = PlanCorpus::standard_mock_registry();
    let catalog = PlanCorpus::standard_mock_catalog();
    std::hint::black_box(
        entries
            .iter()
            .map(|entry| {
                let registry = common::registry_for(entry, &empty, &mock);
                let statement = parse(entry.source).expect("source parses");
                let analyzed = analyze(statement, registry, None).expect("source analyzes");
                let planned = plan(&analyzed, registry).expect("source plans");
                let caps = selene_gql::ImplDefinedCaps::default();
                let ctx = common::context_for(entry, &caps, &catalog);
                optimize(planned, &ctx).pipeline.len()
            })
            .sum(),
    )
}

#[library_benchmark]
fn plan_ir_clone() -> usize {
    let plan = common::representative_plan();
    let cloned = plan.clone();
    std::hint::black_box(cloned.pipeline.len())
}

library_benchmark_group!(
    name = gql_gates;
    benchmarks = parse_corpus, analyze_corpus, plan_optimize_corpus, plan_ir_clone
);

fn gate_config() -> LibraryBenchmarkConfig {
    let mut callgrind = Callgrind::default();
    callgrind.soft_limits([(EventKind::Ir, 5.0)]);
    callgrind.fail_fast(true);

    let mut config = LibraryBenchmarkConfig::default();
    config.tool(callgrind);
    config
}

main!(
    config = gate_config();
    library_benchmark_groups = gql_gates
);
