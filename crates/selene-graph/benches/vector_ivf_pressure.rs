#![allow(missing_docs)]
//! Criterion benches for production IVF candidate-pressure diagnostics.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;
mod single_graph_ann_recall;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_graph::VectorIndexMemoryUsage;
use selene_testing::BenchProfile;
use single_graph_ann_recall::{AnnRecallFixture, AnnRecallProfile, AnnRecallVariant};

const K: usize = 10;
const QUERY_COUNT: usize = 16;
const PROFILES: [AnnRecallProfile; 1] = [AnnRecallProfile::ClusteredCosine];

fn bench_ivf_candidate_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_ivf_candidate_pressure");
    for scale in vector_scales() {
        for profile in PROFILES {
            let variant = ivf_variant(profile);
            let fixture = AnnRecallFixture::build(profile, variant, scale, QUERY_COUNT, K);
            let usage = fixture.memory_usage();
            for &width in variant.search_widths() {
                let recall = recall_basis_points(fixture.mean_recall(width));
                let quality = recall_basis_points(fixture.mean_distance_quality(width));
                group.throughput(Throughput::Elements(estimated_candidates_per_batch(
                    usage,
                    width,
                    fixture.query_count(),
                ) as u64));
                group.bench_function(
                    BenchmarkId::new(
                        profile.name(),
                        format!(
                            "d{}_k{K}_w{width}_idbp{recall}_dqbp{quality}_{}",
                            fixture.dimension(),
                            pressure_suffix(usage, width)
                        ),
                    ),
                    |b| {
                        b.iter(|| {
                            std::hint::black_box(fixture.total_overlap(width));
                        });
                    },
                );
            }
        }
    }
    group.finish();
}

fn ivf_variant(profile: AnnRecallProfile) -> AnnRecallVariant {
    *profile
        .variants()
        .iter()
        .find(|variant| variant.name_suffix == "ivf")
        .expect("ANN recall profile includes an IVF variant")
}

fn vector_scales() -> Vec<usize> {
    std::env::var("SELENE_VECTOR_BENCH_SCALES")
        .ok()
        .and_then(parse_scales)
        .unwrap_or_else(|| BenchProfile::from_env().scales().to_vec())
}

fn parse_scales(raw: String) -> Option<Vec<usize>> {
    let mut scales: Vec<_> = raw
        .split(',')
        .filter_map(|part| part.trim().parse::<usize>().ok())
        .filter(|scale| *scale > 0)
        .collect();
    scales.sort_unstable();
    scales.dedup();
    (!scales.is_empty()).then_some(scales)
}

fn pressure_suffix(usage: VectorIndexMemoryUsage, width: usize) -> String {
    format!(
        "lists{}ne{}max{}avg{}avgq{}maxq{}m{}-{}",
        compact_usize(usage.ivf_list_count),
        compact_usize(usage.ivf_non_empty_list_count),
        compact_usize(usage.ivf_max_list_len),
        compact_usize(average_list_len(usage)),
        compact_usize(estimated_candidates_per_query(usage, width)),
        compact_usize(worst_case_candidates_per_query(usage, width)),
        usage.estimated_index_bytes / 1024,
        usage.estimated_reachable_bytes / 1024,
    )
}

fn estimated_candidates_per_batch(
    usage: VectorIndexMemoryUsage,
    width: usize,
    query_count: usize,
) -> usize {
    estimated_candidates_per_query(usage, width)
        .saturating_mul(query_count)
        .max(1)
}

fn estimated_candidates_per_query(usage: VectorIndexMemoryUsage, width: usize) -> usize {
    usage
        .ivf_average_list_len_basis_points
        .saturating_mul(probe_count(usage, width))
        .saturating_add(9_999)
        / 10_000
}

fn worst_case_candidates_per_query(usage: VectorIndexMemoryUsage, width: usize) -> usize {
    usage
        .ivf_max_list_len
        .saturating_mul(probe_count(usage, width))
}

fn average_list_len(usage: VectorIndexMemoryUsage) -> usize {
    usage
        .ivf_average_list_len_basis_points
        .saturating_add(9_999)
        / 10_000
}

fn probe_count(usage: VectorIndexMemoryUsage, width: usize) -> usize {
    width.max(1).min(usage.ivf_list_count)
}

fn compact_usize(value: usize) -> String {
    compact_count(u64::try_from(value).unwrap_or(u64::MAX))
}

fn compact_count(value: u64) -> String {
    if value >= 1_000 && value.is_multiple_of(1_000) {
        format!("{}k", value / 1_000)
    } else {
        value.to_string()
    }
}

fn recall_basis_points(recall: f64) -> u64 {
    (recall * 10_000.0).round() as u64
}

criterion_group! {
    name = vector_ivf_pressure;
    config = common::criterion_config();
    targets = bench_ivf_candidate_pressure
}
criterion_main!(vector_ivf_pressure);
