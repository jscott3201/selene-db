#![allow(missing_docs)]
//! Criterion benches for adjacency traversal.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::collections::{HashSet, VecDeque};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use selene_core::NodeId;
use selene_graph::SeleneGraph;

fn bench_bfs_depths(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_bfs");
    for depth in [1_usize, 10, 50] {
        for fixture in common::fixtures() {
            group.bench_with_input(
                BenchmarkId::new(format!("d{depth}"), fixture.scale()),
                &fixture,
                |b, fixture| {
                    b.iter(|| {
                        let count = bfs_count(fixture.graph(), fixture.sample_node_id(), depth);
                        std::hint::black_box(count);
                    });
                },
            );
        }
    }
    group.finish();
}

fn bfs_count(graph: &SeleneGraph, start: NodeId, max_depth: usize) -> usize {
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    seen.insert(start);
    queue.push_back((start, 0_usize));
    while let Some((node, depth)) = queue.pop_front() {
        if depth == max_depth {
            continue;
        }
        if let Some(edges) = graph.outgoing_edges(node) {
            for edge in edges.iter() {
                if seen.insert(edge.neighbor) {
                    queue.push_back((edge.neighbor, depth + 1));
                }
            }
        }
    }
    seen.len()
}

criterion_group! {
    name = graph_bfs_group;
    config = common::criterion_config();
    targets = bench_bfs_depths
}
criterion_main!(graph_bfs_group);
