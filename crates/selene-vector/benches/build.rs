#![allow(missing_docs)]
//! Criterion cold-build benchmark for BRIEF-91 HNSW construction timing.

use std::sync::Arc;
use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use selene_core::NodeId;
use selene_vector::{DistanceMetric, HnswConfig, HnswGraph, HnswParams, insert_node, random_layer};

const BUILD_SEED: u64 = 0x9100_0001_u64;
const DIM: usize = 16;
const M: usize = 8;
const EF_CONSTRUCTION: usize = 64;
const EF_SEARCH: usize = 50;

fn bench_hnsw_build(c: &mut Criterion) {
    let config = build_config();
    let mut group = c.benchmark_group("vector_hnsw_build");

    for n in [100_usize, 1_000, 5_000] {
        group.bench_function(BenchmarkId::from_parameter(n), |b| {
            b.iter_batched(
                || build_corpus(n, DIM, BUILD_SEED),
                |corpus| {
                    let mut graph = HnswGraph::with_capacity(DIM as u16, n);
                    let params = HnswParams::from_config(&config);
                    for (node_id, vector, max_layer) in &corpus {
                        insert_node(
                            &mut graph,
                            *node_id,
                            Arc::clone(vector),
                            *max_layer,
                            &params,
                        )
                        .expect("insert succeeds");
                    }
                    std::hint::black_box(graph)
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn build_config() -> HnswConfig {
    HnswConfig::with_params(DIM, M, EF_CONSTRUCTION, EF_SEARCH, DistanceMetric::L2)
        .expect("build bench config is valid")
}

fn build_corpus(n: usize, dim: usize, seed: u64) -> Vec<(NodeId, Arc<[f32]>, u8)> {
    let mut rng = fastrand::Rng::with_seed(seed);
    let params = HnswParams::from_config(&build_config());

    (0..n)
        .map(|idx| {
            let vector = (0..dim)
                .map(|_| (rng.f32() * 2.0) - 1.0)
                .collect::<Vec<_>>();
            let max_layer = random_layer(&mut rng, params.level_factor);
            (NodeId::new((idx + 1) as u64), Arc::from(vector), max_layer)
        })
        .collect()
}

fn criterion_config() -> Criterion {
    let quick = std::env::var("SELENE_BENCH_PROFILE")
        .ok()
        .is_none_or(|profile| {
            !profile.eq_ignore_ascii_case("full") && !profile.eq_ignore_ascii_case("stress")
        });
    Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_millis(100))
        .measurement_time(Duration::from_secs(if quick { 5 } else { 30 }))
}

criterion_group! {
    name = build_group;
    config = criterion_config();
    targets = bench_hnsw_build
}
criterion_main!(build_group);
