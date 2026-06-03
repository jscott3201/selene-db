#![allow(missing_docs)]
//! Gating bench for lock-free reads under write contention (D10, P2).
//!
//! `concurrent_read` measures readers with **no** writer; `concurrent_writers`
//! measures writer commit throughput with background readers (the readers are
//! untimed noise there). Neither times the *reads themselves* while a writer is
//! actively churning the `ArcSwap` snapshot — which is exactly the D10 promise:
//! a held write lock must never block or slow a reader. This bench times a fixed
//! batch of reads across `READER_THREADS` while one background writer commits in
//! a loop, so a regression that accidentally puts reads behind the write lock
//! (or adds reader-side contention) shows up as collapsed read throughput.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{LabelDiff, NodeId, PropertyDiff, Value, intern};
use selene_graph::SharedGraph;

/// Reader threads issuing the timed read batch.
const READER_THREADS: usize = 8;
/// Reads each reader thread performs per sample (total = this × threads).
const READS_PER_THREAD: usize = 20_000;

fn bench_read_under_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_read_under_write");
    let total_reads = (READER_THREADS * READS_PER_THREAD) as u64;
    for fixture in common::fixtures() {
        let scale = fixture.graph().node_count();
        group.throughput(Throughput::Elements(total_reads));
        group.bench_with_input(
            BenchmarkId::from_parameter(scale),
            &fixture,
            |b, fixture| {
                b.iter_batched(
                    || SharedGraph::from_graph(fixture.graph().clone()),
                    |shared| {
                        let writer_done = AtomicBool::new(false);
                        std::thread::scope(|scope| {
                            // One background writer churns commits on node 1 to keep
                            // the snapshot pointer moving while readers run.
                            let writer = {
                                let writer_done = &writer_done;
                                let shared = &shared;
                                scope.spawn(move || {
                                    let score = intern("score").expect("score key interns");
                                    let mut value = 0_i64;
                                    while !writer_done.load(Ordering::Relaxed) {
                                        let mut txn = shared.begin_write();
                                        {
                                            let mut mutator = txn.mutator();
                                            let diff = PropertyDiff::new(
                                                [(score.clone(), Value::Int(value))],
                                                [],
                                            )
                                            .expect("property diff");
                                            mutator
                                                .update_node(
                                                    NodeId::new(1),
                                                    LabelDiff::new([], [])
                                                        .expect("label diff valid"),
                                                    diff,
                                                )
                                                .expect("writer update succeeds");
                                        }
                                        txn.commit().expect("writer commit succeeds");
                                        value = value.wrapping_add(1);
                                    }
                                })
                            };

                            // Timed work: each reader resolves node 1 repeatedly off
                            // the lock-free snapshot while the writer above contends.
                            let readers = (0..READER_THREADS)
                                .map(|_| {
                                    let shared = &shared;
                                    scope.spawn(move || {
                                        let mut hits = 0_usize;
                                        for _ in 0..READS_PER_THREAD {
                                            let snapshot = shared.read();
                                            if snapshot.node_properties(NodeId::new(1)).is_some() {
                                                hits += 1;
                                            }
                                        }
                                        black_box(hits)
                                    })
                                })
                                .collect::<Vec<_>>();

                            for reader in readers {
                                reader.join().expect("reader thread joins");
                            }
                            // Readers are done — stop the writer and reclaim it
                            // (its final commit lands in untimed teardown).
                            writer_done.store(true, Ordering::Relaxed);
                            writer.join().expect("writer thread joins");
                        });
                        black_box(shared.read().node_count())
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

criterion_group! {
    name = graph_read_under_write;
    config = common::criterion_config();
    targets = bench_read_under_write
}
criterion_main!(graph_read_under_write);
