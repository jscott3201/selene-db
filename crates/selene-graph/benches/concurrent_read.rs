#![allow(missing_docs)]
//! Criterion bench for concurrent lock-free snapshot reads.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_graph::SharedGraph;

const LEGACY_THREADS: usize = 10;
const SINGLE_THREAD_READS: usize = 100_000;
const PARALLEL_THREADS: usize = 8;
const PARALLEL_READS_PER_THREAD: usize = 20_000;

fn bench_concurrent_reads(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_concurrent_reads");
    for fixture in common::fixtures() {
        let shared = SharedGraph::from_graph(fixture.graph().clone());
        group.throughput(Throughput::Elements(LEGACY_THREADS as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(fixture.scale()),
            &shared,
            |b, shared| {
                b.iter(|| {
                    std::thread::scope(|scope| {
                        let handles = (0..LEGACY_THREADS)
                            .map(|_| {
                                scope.spawn(|| {
                                    let snapshot = shared.read();
                                    std::hint::black_box(snapshot.node_count());
                                })
                            })
                            .collect::<Vec<_>>();
                        for handle in handles {
                            handle.join().expect("reader thread joins");
                        }
                    });
                });
            },
        );
    }
    group.finish();
}

fn bench_snapshot_read_loops(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_snapshot_read_loops");
    for fixture in common::fixtures() {
        let shared = SharedGraph::from_graph(fixture.graph().clone());
        group.throughput(Throughput::Elements(SINGLE_THREAD_READS as u64));
        group.bench_with_input(
            BenchmarkId::new("single_thread", fixture.scale()),
            &shared,
            |b, shared| {
                b.iter(|| {
                    let mut observed = 0_usize;
                    for _ in 0..SINGLE_THREAD_READS {
                        let snapshot = shared.read();
                        observed ^= snapshot.node_count();
                    }
                    std::hint::black_box(observed);
                });
            },
        );

        let total_parallel_reads = (PARALLEL_THREADS * PARALLEL_READS_PER_THREAD) as u64;
        group.throughput(Throughput::Elements(total_parallel_reads));
        group.bench_with_input(
            BenchmarkId::new("parallel_threads8", fixture.scale()),
            &shared,
            |b, shared| {
                b.iter(|| {
                    std::thread::scope(|scope| {
                        let handles = (0..PARALLEL_THREADS)
                            .map(|_| {
                                scope.spawn(|| {
                                    let mut observed = 0_usize;
                                    for _ in 0..PARALLEL_READS_PER_THREAD {
                                        let snapshot = shared.read();
                                        observed ^= snapshot.node_count();
                                    }
                                    std::hint::black_box(observed);
                                })
                            })
                            .collect::<Vec<_>>();
                        for handle in handles {
                            handle.join().expect("reader thread joins");
                        }
                    });
                });
            },
        );
    }
    group.finish();
}

criterion_group! {
    name = graph_concurrency;
    config = common::criterion_config();
    targets = bench_concurrent_reads, bench_snapshot_read_loops
}
criterion_main!(graph_concurrency);
