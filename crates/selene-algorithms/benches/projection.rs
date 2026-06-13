#![allow(missing_docs)]
//! Gating benches for the `GraphProjection` CSR foundation (ALGO-01/02/05).
//!
//! Every algorithm runs *over* a projection, but the existing `algo_bench`
//! builds that projection in untimed setup — so the build cost (graph scan +
//! CSR construction) and the per-edge neighbor-iteration cost were invisible.
//! ALGO-01/02/05 reshape the CSR index (dense `u32` columns), which changes
//! exactly these two numbers. This bench isolates them:
//!
//! - `algo/projection_build` — full `GraphProjection::build` over a fixture.
//! - `algo/projection_neighbor_iter` — sweep every node's out-neighbor slice on
//!   a pre-built projection, summing a field so the slice walk is not elided.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_testing::{BenchFixture, BenchProfile};

use common::{build_projection, criterion_config, projection_config, scale_label};

fn bench_projection_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/projection_build");
    for &scale in BenchProfile::from_env().scales() {
        // Fixture build is untimed; only the projection construction is sampled.
        let fixture = BenchFixture::build(scale);
        let snapshot = fixture.graph();
        group.throughput(Throughput::Elements(snapshot.node_count() as u64));
        group.bench_function(BenchmarkId::from_parameter(scale_label(scale)), |b| {
            let config = projection_config();
            b.iter(|| {
                black_box(
                    selene_algorithms::GraphProjection::build(snapshot, &config, None)
                        .expect("bench projection builds"),
                )
            });
        });
    }
    group.finish();
}

fn bench_projection_neighbor_iter(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/projection_neighbor_iter");
    for &scale in BenchProfile::from_env().scales() {
        let fixture = BenchFixture::build(scale);
        let projection = build_projection(fixture.graph());
        // Throughput keyed on edges: the neighbor walk visits every out-edge.
        group.throughput(Throughput::Elements(projection.edge_count() as u64));
        group.bench_function(BenchmarkId::from_parameter(scale_label(scale)), |b| {
            b.iter(|| {
                let mut acc = 0.0_f64;
                for node in projection.iter_nodes() {
                    for neighbor in projection.out_neighbors(node) {
                        acc += neighbor.weight;
                    }
                }
                black_box(acc)
            });
        });
    }
    group.finish();
}

criterion_group! {
    name = projection;
    config = criterion_config();
    targets = bench_projection_build, bench_projection_neighbor_iter
}
criterion_main!(projection);
