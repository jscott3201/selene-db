#![allow(missing_docs)]
//! Benchmark-only wgpu prototype for batched vector scoring.
//!
//! This is not a production accelerator. It measures realistic GPU envelopes:
//! candidate vectors stay resident, query batches may be rewritten, compute
//! shaders score every query/candidate pair, and readback can be measured as
//! either all scores or reduced block-local top-k candidates.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[path = "vector_wgpu/case.rs"]
mod vector_wgpu_case;
#[path = "vector_wgpu/fixture.rs"]
mod vector_wgpu_fixture;
#[path = "vector_wgpu/pipeline.rs"]
mod vector_wgpu_pipeline;
#[path = "vector_wgpu/shader.rs"]
mod vector_wgpu_shader;
#[path = "vector_wgpu/support.rs"]
mod vector_wgpu_support;

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};

use vector_wgpu_case::cases;
use vector_wgpu_support::WgpuBench;

fn bench_config() -> Criterion {
    let (samples, ms) = match std::env::var("SELENE_BENCH_PROFILE").ok().as_deref() {
        Some("full") | Some("stress") => (30usize, 1_500u64),
        _ => (10, 500),
    };
    Criterion::default()
        .sample_size(samples)
        .warm_up_time(std::time::Duration::from_millis(100))
        .measurement_time(std::time::Duration::from_millis(ms))
}

#[allow(clippy::print_stderr)]
fn bench_vector_wgpu(c: &mut Criterion) {
    let mut group = c.benchmark_group("core_vector_wgpu_prototype");
    for case in cases() {
        let mut bench = match pollster::block_on(WgpuBench::build(case)) {
            Ok(bench) => bench,
            Err(error) => {
                eprintln!(
                    "[core_vector_wgpu_prototype] skipping q{}x{}x{}: {error}",
                    case.queries, case.candidates, case.dimension
                );
                continue;
            }
        };
        let mut scores = vec![0.0f32; case.score_count()];
        let mut partial_distances = vec![0.0f32; case.partial_count()];
        let mut partial_indices = vec![0u32; case.partial_count()];
        group.throughput(Throughput::Elements(case.score_count() as u64));
        group.bench_function(case.id("resident_query_copy_score_readback"), |b| {
            b.iter(|| {
                bench
                    .score_with_query_write(black_box(&mut scores))
                    .expect("wgpu scoring succeeds")
            });
        });
        group.bench_function(case.id("resident_preloaded_score_readback"), |b| {
            b.iter(|| {
                bench
                    .score_preloaded(black_box(&mut scores))
                    .expect("wgpu scoring succeeds")
            });
        });
        group.bench_function(case.id("cold_candidate_upload_score_readback"), |b| {
            b.iter(|| {
                bench
                    .score_with_candidate_upload(black_box(&mut scores))
                    .expect("wgpu scoring succeeds")
            });
        });
        group.bench_function(
            case.id("resident_query_copy_score_readback_cpu_topk"),
            |b| {
                b.iter(|| {
                    bench
                        .score_with_query_write_top_k(black_box(&mut scores))
                        .expect("wgpu scoring succeeds")
                });
            },
        );
        group.bench_function(case.id("cpu_rayon_score_topk"), |b| {
            b.iter(|| black_box(bench.cpu_parallel_score_top_k()));
        });
        group.bench_function(
            case.id("resident_query_copy_score_gpu_block_topk_cpu_merge"),
            |b| {
                b.iter(|| {
                    bench
                        .score_with_query_write_block_top_k(
                            black_box(&mut partial_distances),
                            black_box(&mut partial_indices),
                        )
                        .expect("wgpu block top-k succeeds")
                });
            },
        );
        group.bench_function(
            case.id("resident_query_copy_score_fused_block_topk_cpu_merge"),
            |b| {
                b.iter(|| {
                    bench
                        .score_with_query_write_fused_block_top_k(
                            black_box(&mut partial_distances),
                            black_box(&mut partial_indices),
                        )
                        .expect("wgpu fused block top-k succeeds")
                });
            },
        );
    }
    group.finish();
}

criterion_group! {
    name = vector_wgpu;
    config = bench_config();
    targets = bench_vector_wgpu
}
criterion_main!(vector_wgpu);
