#![allow(missing_docs)]
//! Criterion benches for graph mutation commit hot paths.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{LabelSet, PropertyDiff, PropertyMap, Value};
use selene_graph::SharedGraph;

fn bench_edge_create_cascade(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_edge_create_cascade");
    for fixture in common::fixtures() {
        group.throughput(Throughput::Elements(1));
        group.bench_function(BenchmarkId::from_parameter(fixture.scale()), |b| {
            b.iter_batched(
                || SharedGraph::from_graph(fixture.graph().clone()),
                |shared| {
                    let mut txn = shared.begin_write();
                    {
                        let mut mutator = txn.mutator();
                        let created = mutator
                            .create_node(
                                LabelSet::single(fixture.person_label()),
                                PropertyMap::new(),
                            )
                            .expect("node create succeeds");
                        mutator
                            .create_edge(
                                fixture.edge_label(),
                                fixture.sample_node_id(),
                                created,
                                PropertyMap::new(),
                            )
                            .expect("edge create succeeds");
                        mutator
                            .delete_node(fixture.sample_node_id())
                            .expect("cascade delete succeeds");
                    }
                    let changes = txn.commit().expect("commit succeeds").changes.len();
                    // Return the per-iteration graph so Criterion drops it
                    // after timing; this benchmark measures mutation +
                    // commit, not fixture teardown.
                    std::hint::black_box((shared, changes))
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_mutation_commit_batches(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_mutation_commit_batch");
    for batch in [10_usize, 100, 1_000] {
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
                                for idx in 0..batch {
                                    let node = selene_core::NodeId::new(
                                        (idx % fixture.scale()) as u64 + 1,
                                    );
                                    let diff = PropertyDiff::new(
                                        [(
                                            fixture.score_key(),
                                            Value::Int((idx as i64).wrapping_mul(17)),
                                        )],
                                        [],
                                    )
                                    .expect("property diff is valid");
                                    mutator
                                        .update_node(
                                            node,
                                            selene_core::LabelDiff::new([], []).unwrap(),
                                            diff,
                                        )
                                        .expect("node update succeeds");
                                }
                            }
                            let changes = txn.commit().expect("commit succeeds").changes.len();
                            // Return the per-iteration graph so Criterion
                            // drops it after timing; this benchmark measures
                            // mutation + commit, not fixture teardown.
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
    name = graph_mutations;
    config = common::criterion_config();
    targets = bench_edge_create_cascade, bench_mutation_commit_batches
}
criterion_main!(graph_mutations);
