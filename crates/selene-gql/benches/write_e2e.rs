#![allow(missing_docs)]
//! Criterion benches for write-path GQL and direct durable mutation flows.
//!
//! Two families. The in-memory CPU family (`gql_*` arms) runs on a no-WAL
//! `SharedGraph` (`gql_write_state_in_memory`) so it isolates parse, plan,
//! execute, and in-memory commit CPU — the deltas GQLRT-05 / CORE-06 move. The
//! durable family (`*_with_flush`, `direct_*`) keeps a real WAL. Note that a
//! WAL-backed `SharedGraph` ALWAYS commits in `OnFlushOnly` with
//! `CommitBatching::Off` (the committer owns fsync since v1.1), so any caller
//! `SyncPolicy` is inert — which is exactly why the CPU arms must not be
//! WAL-backed (one fsync per commit would swamp the CPU sample).

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::num::NonZeroUsize;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::intern;
use selene_gql::{EmptyProcedureRegistry, Session, StatementOutput};
use selene_graph::TypedIndexKind;
use selene_persist::SyncPolicy;
use selene_testing::{BenchProfile, WriteCorpus};

fn bench_write_e2e(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_e2e");
    for &scale in BenchProfile::from_env().scales() {
        bench_gql_per_iter_plan(&mut group, scale);
        bench_gql_fresh_preplanned(
            &mut group,
            "gql_insert_single_node_preplanned",
            scale,
            WriteCorpus::insert_single_node(),
        );
        bench_gql_insert_single_node_cached(&mut group, scale);
        bench_gql_insert_single_node_cached_with_schema_churn(&mut group, scale);
        bench_gql_fresh_preplanned_with_flush(
            &mut group,
            "gql_insert_single_node_preplanned_with_flush",
            scale,
            WriteCorpus::insert_single_node(),
        );
        bench_gql_fresh_preplanned(
            &mut group,
            "gql_insert_node_with_edge_preplanned",
            scale,
            WriteCorpus::insert_node_with_edge(),
        );
        bench_gql_fresh_preplanned(
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
        bench_explicit_txn_3_inserts_rust_api(&mut group, scale);
        bench_explicit_txn_3_inserts_rollback(&mut group, scale);
        bench_direct_flush(&mut group, scale);
        bench_direct_flush_every10(&mut group, scale);
    }
    group.finish();
}

fn bench_gql_insert_single_node_cached(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scale: usize,
) {
    group.throughput(Throughput::Elements(1));
    let state = common::gql_write_state_in_memory(scale);
    let registry = EmptyProcedureRegistry;
    let source = WriteCorpus::insert_single_node();
    let mut session =
        Session::new(&state.graph).with_plan_cache(NonZeroUsize::new(64).expect("nonzero"));
    execute_cached_source(&mut session, source, &registry);
    group.bench_function(
        BenchmarkId::new("gql_insert_single_node_cached", scale),
        |b| {
            b.iter(|| {
                let rows = execute_cached_source(&mut session, source, &registry);
                std::hint::black_box(rows)
            });
        },
    );
    std::hint::black_box(&state);
}

fn bench_gql_insert_single_node_cached_with_schema_churn(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scale: usize,
) {
    group.throughput(Throughput::Elements(1));
    let state = common::gql_write_state_in_memory(scale);
    let registry = EmptyProcedureRegistry;
    let source = WriteCorpus::insert_single_node();
    let label = intern("Person").expect("Person label interns");
    let property = intern("plan_cache_churn").expect("churn property interns");
    let mut session =
        Session::new(&state.graph).with_plan_cache(NonZeroUsize::new(64).expect("nonzero"));
    execute_cached_source(&mut session, source, &registry);
    let mut iteration = 0_u64;
    let mut index_exists = false;
    group.bench_function(
        BenchmarkId::new("gql_insert_single_node_cached_with_schema_churn", scale),
        |b| {
            b.iter(|| {
                if iteration.is_multiple_of(10) {
                    if index_exists {
                        state
                            .graph
                            .drop_property_index(label.clone(), property.clone())
                            .expect("churn index drops");
                    } else {
                        state
                            .graph
                            .create_property_index(
                                label.clone(),
                                property.clone(),
                                TypedIndexKind::I64,
                            )
                            .expect("churn index creates");
                    }
                    index_exists = !index_exists;
                }
                iteration = iteration.saturating_add(1);
                let rows = execute_cached_source(&mut session, source, &registry);
                std::hint::black_box(rows)
            });
        },
    );
    std::hint::black_box(&state);
}

fn execute_cached_source(
    session: &mut Session<'_>,
    source: &str,
    registry: &EmptyProcedureRegistry,
) -> usize {
    match session
        .execute_source(source, registry)
        .expect("cached write statement executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        StatementOutput::Written(outcome) => {
            let rows = outcome.rows.as_ref().map_or(0, |table| table.row_count());
            rows + outcome.changes.len()
        }
        StatementOutput::Empty => 0,
        _ => panic!("unexpected statement output"),
    }
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
                || common::gql_write_state_in_memory(scale),
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
            || common::gql_write_state_in_memory(scale),
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

fn bench_gql_fresh_preplanned_with_flush(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &'static str,
    scale: usize,
    source: &str,
) {
    group.throughput(Throughput::Elements(1));
    let plan = common::plan_write(source);
    group.bench_function(BenchmarkId::new(name, scale), |b| {
        b.iter_batched(
            || common::gql_write_state(scale, SyncPolicy::OnFlushOnly),
            |state| {
                let mut session = Session::new(&state.graph);
                let rows = common::execute_preplanned(&plan, &mut session);
                let durable_at = session.flush().expect("flush succeeds");
                drop(session);
                std::hint::black_box((state, rows, durable_at))
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
    let plans = WriteCorpus::multi_statement_txn()
        .iter()
        .map(|source| common::plan_write(source))
        .collect::<Vec<_>>();
    group.bench_function(
        BenchmarkId::new("gql_multi_statement_txn_preplanned", scale),
        |b| {
            b.iter_batched(
                || common::gql_write_state_in_memory(scale),
                |state| {
                    let mut session = Session::new(&state.graph);
                    let rows = plans
                        .iter()
                        .map(|plan| common::execute_preplanned(plan, &mut session))
                        .sum::<usize>();
                    drop(session);
                    std::hint::black_box((state, rows))
                },
                BatchSize::LargeInput,
            );
        },
    );
}

fn bench_explicit_txn_3_inserts_rust_api(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scale: usize,
) {
    group.throughput(Throughput::Elements(3));
    let plans = WriteCorpus::multi_statement_txn()[1..4]
        .iter()
        .map(|source| common::plan_write(source))
        .collect::<Vec<_>>();
    group.bench_function(
        BenchmarkId::new("explicit_txn_3_inserts_rust_api", scale),
        |b| {
            b.iter_batched(
                || common::gql_write_state_in_memory(scale),
                |state| {
                    let mut session = Session::new(&state.graph);
                    session.start_transaction().expect("start succeeds");
                    let rows = plans
                        .iter()
                        .map(|plan| common::execute_preplanned(plan, &mut session))
                        .sum::<usize>();
                    let outcome = session.commit_transaction().expect("commit succeeds");
                    drop(session);
                    std::hint::black_box((state, rows + outcome.changes.len()))
                },
                BatchSize::LargeInput,
            );
        },
    );
}

fn bench_explicit_txn_3_inserts_rollback(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scale: usize,
) {
    group.throughput(Throughput::Elements(3));
    let plans = WriteCorpus::multi_statement_txn()[1..4]
        .iter()
        .map(|source| common::plan_write(source))
        .collect::<Vec<_>>();
    group.bench_function(
        BenchmarkId::new("explicit_txn_3_inserts_rollback", scale),
        |b| {
            b.iter_batched(
                || common::gql_write_state_in_memory(scale),
                |state| {
                    let mut session = Session::new(&state.graph);
                    session.start_transaction().expect("start succeeds");
                    let rows = plans
                        .iter()
                        .map(|plan| common::execute_preplanned(plan, &mut session))
                        .sum::<usize>();
                    let outcome = session.rollback_transaction().expect("rollback succeeds");
                    drop(session);
                    std::hint::black_box((state, rows + outcome.discarded_changes))
                },
                BatchSize::LargeInput,
            );
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
