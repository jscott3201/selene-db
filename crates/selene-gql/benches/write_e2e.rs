#![allow(missing_docs)]
//! Criterion benches for write-path GQL and direct durable mutation flows.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_gql::Session;
use selene_persist::SyncPolicy;
use selene_testing::{BenchProfile, WriteCorpus};

fn bench_write_e2e(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_e2e");
    for &scale in BenchProfile::from_env().scales() {
        bench_gql_per_iter_plan(&mut group, scale);
        bench_gql_reused_preplanned(
            &mut group,
            "gql_insert_single_node_preplanned",
            scale,
            WriteCorpus::insert_single_node(),
        );
        bench_gql_reused_preplanned(
            &mut group,
            "gql_insert_node_with_edge_preplanned",
            scale,
            WriteCorpus::insert_node_with_edge(),
        );
        bench_gql_reused_preplanned(
            &mut group,
            "gql_match_set_preplanned",
            scale,
            WriteCorpus::match_set(),
        );
        bench_gql_fresh_preplanned(
            &mut group,
            "gql_match_delete_preplanned",
            scale,
            WriteCorpus::match_delete(),
        );
        bench_gql_multi_statement(&mut group, scale);
        bench_direct_flush(&mut group, scale);
        bench_direct_flush_every10(&mut group, scale);
    }
    group.finish();
}

fn bench_gql_per_iter_plan(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scale: usize,
) {
    group.throughput(Throughput::Elements(1));
    group.bench_function(
        BenchmarkId::new("gql_insert_single_node_per_iter_plan", scale),
        |b| {
            b.iter_batched(
                || common::gql_write_state(scale, SyncPolicy::EveryN(1_000)),
                |state| {
                    let plan = common::plan_write(WriteCorpus::insert_single_node());
                    let mut session = Session::new(&state.graph);
                    let rows = common::execute_preplanned(&plan, &mut session);
                    drop(session);
                    std::hint::black_box((state, rows))
                },
                BatchSize::LargeInput,
            );
        },
    );
}

fn bench_gql_reused_preplanned(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &'static str,
    scale: usize,
    source: &str,
) {
    group.throughput(Throughput::Elements(1));
    let state = common::gql_write_state(scale, SyncPolicy::EveryN(1_000));
    let plan = common::plan_write(source);
    let mut session = Session::new(&state.graph);
    group.bench_function(BenchmarkId::new(name, scale), |b| {
        b.iter(|| std::hint::black_box(common::execute_preplanned(&plan, &mut session)));
    });
}

fn bench_gql_fresh_preplanned(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &'static str,
    scale: usize,
    source: &str,
) {
    group.throughput(Throughput::Elements(1));
    let plan = common::plan_write(source);
    group.bench_function(BenchmarkId::new(name, scale), |b| {
        b.iter_batched(
            || common::gql_write_state(scale, SyncPolicy::EveryN(1_000)),
            |state| {
                let mut session = Session::new(&state.graph);
                let rows = common::execute_preplanned(&plan, &mut session);
                drop(session);
                std::hint::black_box((state, rows))
            },
            BatchSize::LargeInput,
        );
    });
}

fn bench_gql_multi_statement(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scale: usize,
) {
    group.throughput(Throughput::Elements(3));
    let state = common::gql_write_state(scale, SyncPolicy::EveryN(1_000));
    let plans = WriteCorpus::multi_statement_txn()
        .iter()
        .map(|source| common::plan_write(source))
        .collect::<Vec<_>>();
    let mut session = Session::new(&state.graph);
    group.bench_function(
        BenchmarkId::new("gql_multi_statement_txn_preplanned", scale),
        |b| {
            b.iter(|| {
                let rows = plans
                    .iter()
                    .map(|plan| common::execute_preplanned(plan, &mut session))
                    .sum::<usize>();
                std::hint::black_box(rows)
            });
        },
    );
}

fn bench_direct_flush(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scale: usize,
) {
    group.throughput(Throughput::Elements(1));
    group.bench_function(
        BenchmarkId::new("direct_insert_single_node_with_wal_flush", scale),
        |b| {
            b.iter_batched(
                || common::direct_write_state(scale, SyncPolicy::OnFlushOnly),
                |mut state| {
                    let changes = common::execute_direct_insert(&mut state, true);
                    std::hint::black_box((state, changes))
                },
                BatchSize::LargeInput,
            );
        },
    );
}

fn bench_direct_flush_every10(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scale: usize,
) {
    group.throughput(Throughput::Elements(10));
    group.bench_function(
        BenchmarkId::new("direct_insert_single_node_with_wal_flush_every10", scale),
        |b| {
            b.iter_batched(
                || common::direct_write_state(scale, SyncPolicy::OnFlushOnly),
                |mut state| {
                    let mut changes = 0;
                    for idx in 0..10 {
                        changes += common::execute_direct_insert(&mut state, idx == 9);
                    }
                    std::hint::black_box((state, changes))
                },
                BatchSize::LargeInput,
            );
        },
    );
}

criterion_group! {
    name = write_e2e;
    config = common::criterion_config();
    targets = bench_write_e2e
}
criterion_main!(write_e2e);
