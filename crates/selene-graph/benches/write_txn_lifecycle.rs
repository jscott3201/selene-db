#![allow(missing_docs)]
//! Criterion benches for isolated write-transaction lifecycle costs.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{LabelSet, NodeId, PropertyMap, db_string};
use selene_graph::SharedGraph;

const BATCHES: &[usize] = &[1, 10, 100, 1_000];

fn bench_empty_commit(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_txn_lifecycle/empty_commit");
    group.throughput(Throughput::Elements(1));
    for fixture in common::fixtures() {
        group.bench_function(BenchmarkId::from_parameter(fixture.scale()), |b| {
            b.iter_batched(
                || SharedGraph::from_graph(fixture.graph().clone()),
                |shared| {
                    let outcome = shared
                        .begin_write()
                        .commit()
                        .expect("empty commit succeeds");
                    std::hint::black_box((shared, outcome.generation))
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_create_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_txn_lifecycle/create_only");
    let label =
        LabelSet::single(db_string("BenchCreated").expect("bench label fits DB string cap"));
    for &batch in BATCHES {
        for fixture in common::fixtures() {
            group.throughput(Throughput::Elements(batch as u64));
            group.bench_function(
                BenchmarkId::new(format!("n{}", fixture.scale()), batch),
                |b| {
                    b.iter_batched(
                        || SharedGraph::from_graph(fixture.graph().clone()),
                        |shared| {
                            let mut txn = shared.begin_write();
                            {
                                let mut mutator = txn.mutator();
                                for _ in 0..batch {
                                    mutator
                                        .create_node(label.clone(), PropertyMap::new())
                                        .expect("node create succeeds");
                                }
                            }
                            let changes = txn.commit().expect("commit succeeds").changes.len();
                            std::hint::black_box((shared, changes))
                        },
                        BatchSize::SmallInput,
                    );
                },
            );
        }
    }
    group.finish();
}

fn bench_delete_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_txn_lifecycle/delete_only");
    let label =
        LabelSet::single(db_string("BenchDeleted").expect("bench label fits DB string cap"));
    for &batch in BATCHES {
        for fixture in common::fixtures() {
            group.throughput(Throughput::Elements(batch as u64));
            group.bench_function(
                BenchmarkId::new(format!("n{}", fixture.scale()), batch),
                |b| {
                    b.iter_batched(
                        || {
                            let shared = SharedGraph::from_graph(fixture.graph().clone());
                            let mut ids = Vec::with_capacity(batch);
                            {
                                let mut txn = shared.begin_write();
                                {
                                    let mut mutator = txn.mutator();
                                    for _ in 0..batch {
                                        ids.push(
                                            mutator
                                                .create_node(label.clone(), PropertyMap::new())
                                                .expect("seed node create succeeds"),
                                        );
                                    }
                                }
                                txn.commit().expect("seed commit succeeds");
                            }
                            (shared, ids)
                        },
                        |(shared, ids): (SharedGraph, Vec<NodeId>)| {
                            let mut txn = shared.begin_write();
                            {
                                let mut mutator = txn.mutator();
                                for id in ids {
                                    mutator.delete_node(id).expect("node delete succeeds");
                                }
                            }
                            let changes = txn.commit().expect("commit succeeds").changes.len();
                            std::hint::black_box((shared, changes))
                        },
                        BatchSize::SmallInput,
                    );
                },
            );
        }
    }
    group.finish();
}

criterion_group! {
    name = write_txn_lifecycle;
    config = common::criterion_config();
    targets = bench_empty_commit, bench_create_only, bench_delete_only
}
criterion_main!(write_txn_lifecycle);
