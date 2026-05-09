#![allow(missing_docs)]
//! Criterion bench for concurrent lock-free snapshot reads.

mod common;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_graph::SharedGraph;

fn bench_concurrent_reads(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_concurrent_reads");
    for fixture in common::fixtures() {
        let shared = SharedGraph::from_graph(fixture.graph().clone());
        group.throughput(Throughput::Elements(10));
        group.bench_with_input(
            BenchmarkId::from_parameter(fixture.scale()),
            &shared,
            |b, shared| {
                b.iter(|| {
                    std::thread::scope(|scope| {
                        let handles = (0..10)
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

criterion_group! {
    name = graph_concurrency;
    config = common::criterion_config();
    targets = bench_concurrent_reads
}
criterion_main!(graph_concurrency);
