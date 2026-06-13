//! Local-only oMLX embedding rows for realistic vector distributions.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_testing::local_omlx::{EmbeddingBenchConfig, EmbeddingProvider};

use crate::common::scale_label;

use self::fixture::OmlxVectorFixture;

mod fixture;

const TOP_K: usize = 4;
const ANN_SEARCH_WIDTH: usize = 64;
const IVF_SEARCH_WIDTH: usize = 2;
const TURBO_QUANT_SEARCH_WIDTH: usize = 512;
const ANN_UNION_SEED_K: usize = 8;
const MIXED_READS_PER_CYCLE: usize = 60;
const MIXED_REFRESHES_PER_CYCLE: usize = 40;
const MIXED_ROUNDS_PER_CYCLE: usize = 10;
const MIXED_READS_PER_ROUND: usize = MIXED_READS_PER_CYCLE / MIXED_ROUNDS_PER_CYCLE;
const MIXED_REFRESHES_PER_ROUND: usize = MIXED_REFRESHES_PER_CYCLE / MIXED_ROUNDS_PER_CYCLE;

pub(super) fn bench(c: &mut Criterion) {
    let Some(config) = EmbeddingBenchConfig::from_env() else {
        return;
    };
    let inputs = config.inputs();
    let mut group = c.benchmark_group("graph_vector_omlx_embedding_pressure");
    for model in &config.models {
        let model_id = model_id(model);
        let vectors = config
            .embed(model, &inputs)
            .expect("embedding request succeeds");
        let fixture =
            OmlxVectorFixture::build(model, &inputs, vectors, config.graph_hint_docs_per_topic);
        let topic_precision = precision_basis_points(
            fixture.topic_candidate_total_precision(),
            fixture.query_count() * TOP_K,
        );
        let neighbor_precision = precision_basis_points(
            fixture.topic_neighbor_total_precision(),
            fixture.query_count() * TOP_K,
        );
        let neighbor_batch_precision = precision_basis_points(
            fixture.topic_neighbor_batch_total_precision(),
            fixture.query_count() * TOP_K,
        );
        let label_ann_union_precision = precision_basis_points(
            fixture.topic_label_ann_union_total_precision(),
            fixture.query_count() * TOP_K,
        );
        let neighbor_ann_union_precision = precision_basis_points(
            fixture.topic_neighbor_ann_union_total_precision(),
            fixture.query_count() * TOP_K,
        );
        let hint_expansion_precision = precision_basis_points(
            fixture.topic_hint_expansion_total_precision(),
            fixture.query_count() * TOP_K,
        );
        let hint_expansion_bm25_precision = precision_basis_points(
            fixture.topic_hint_expansion_bm25_total_precision(),
            fixture.query_count() * TOP_K,
        );
        let hint_expansion_bm25_vector_precision = precision_basis_points(
            fixture.topic_hint_expansion_bm25_vector_total_precision(),
            fixture.query_count() * TOP_K,
        );
        let hint_expansion_ann_union_precision = precision_basis_points(
            fixture.topic_hint_expansion_ann_union_total_precision(),
            fixture.query_count() * TOP_K,
        );
        let hint_expansion_cached_precision = precision_basis_points(
            fixture.topic_hint_expansion_cached_total_precision(),
            fixture.query_count() * TOP_K,
        );
        let hint_expansion_state_precision = precision_basis_points(
            fixture.topic_hint_expansion_state_total_precision(),
            fixture.query_count() * TOP_K,
        );
        let ann_hint_expansion_state_precision = precision_basis_points(
            fixture.ann_hint_expansion_state_total_precision(),
            fixture.query_count() * TOP_K,
        );
        let exact_target_hit = fixture.exact_target_hit_basis_points();
        let ann_target_hit = fixture.ann_target_hit_basis_points();
        let turbo_quant_target_hit = fixture.turbo_quant_target_hit_basis_points();
        let ivf_target_hit = fixture.ivf_target_hit_basis_points();
        let hint_expansion_refresh_candidates =
            fixture.topic_hint_expansion_refresh_total_candidates();
        let mixed_cycle_elements = MIXED_READS_PER_CYCLE
            * fixture.query_count()
            * fixture.topic_hint_expansion_cached_count()
            + MIXED_REFRESHES_PER_CYCLE * hint_expansion_refresh_candidates;
        if matches!(config.provider, EmbeddingProvider::Omlx) {
            group.throughput(Throughput::Elements(inputs.len() as u64));
            group.bench_function(
                BenchmarkId::new(
                    "embed_batch",
                    format!(
                        "{}_docs{}_batch{}_dim{}",
                        model_id,
                        inputs.len(),
                        config.batch_size,
                        fixture.dimension
                    ),
                ),
                |b| {
                    b.iter(|| {
                        black_box(
                            config
                                .embed(model, &inputs)
                                .expect("embedding request succeeds"),
                        );
                    });
                },
            );
        }
        group.throughput(Throughput::Elements((fixture.query_count() * TOP_K) as u64));
        group.bench_function(
            BenchmarkId::new(
                "exact_graph_search",
                append_target_hit(
                    format!(
                        "{}_{}_q{}_k{}_dim{}_precbp{}",
                        model_id,
                        scale_label(fixture.document_count()),
                        fixture.query_count(),
                        TOP_K,
                        fixture.dimension,
                        fixture.exact_precision_basis_points(),
                    ),
                    exact_target_hit,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.exact_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "hnsw_graph_search",
                append_target_hit(
                    format!(
                        "{}_{}_q{}_k{}_ef{}_dim{}_precbp{}",
                        model_id,
                        scale_label(fixture.document_count()),
                        fixture.query_count(),
                        TOP_K,
                        ANN_SEARCH_WIDTH,
                        fixture.dimension,
                        fixture.ann_precision_basis_points(),
                    ),
                    ann_target_hit,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.ann_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "turbo_quant_graph_search",
                append_target_hit(
                    format!(
                        "{}_{}_q{}_k{}_c{}_dim{}_precbp{}",
                        model_id,
                        scale_label(fixture.document_count()),
                        fixture.query_count(),
                        TOP_K,
                        TURBO_QUANT_SEARCH_WIDTH,
                        fixture.dimension,
                        fixture.turbo_quant_precision_basis_points(),
                    ),
                    turbo_quant_target_hit,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.turbo_quant_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "ivf_graph_search",
                append_target_hit(
                    format!(
                        "{}_{}_q{}_k{}_p{}_dim{}_precbp{}",
                        model_id,
                        scale_label(fixture.document_count()),
                        fixture.query_count(),
                        TOP_K,
                        IVF_SEARCH_WIDTH,
                        fixture.dimension,
                        fixture.ivf_precision_basis_points(),
                    ),
                    ivf_target_hit,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.ivf_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "topic_label_candidate_score",
                format!(
                    "{}_{}_q{}_k{}_c{}_dim{}_precbp{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    fixture.query_count(),
                    TOP_K,
                    fixture.topic_candidate_count(),
                    fixture.dimension,
                    topic_precision,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.topic_candidate_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "topic_neighbor_score",
                format!(
                    "{}_{}_q{}_k{}_c{}_dim{}_precbp{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    fixture.query_count(),
                    TOP_K,
                    fixture.topic_neighbor_count(),
                    fixture.dimension,
                    neighbor_precision,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.topic_neighbor_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "topic_neighbor_batch_score",
                format!(
                    "{}_{}_q{}_k{}_c{}_dim{}_precbp{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    fixture.query_count(),
                    TOP_K,
                    fixture.topic_neighbor_count(),
                    fixture.dimension,
                    neighbor_batch_precision,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.topic_neighbor_batch_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "topic_label_ann_union_score",
                format!(
                    "{}_{}_precbp{}_q{}_k{}_c{}_ann{}_dim{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    label_ann_union_precision,
                    fixture.query_count(),
                    TOP_K,
                    fixture.topic_label_ann_union_count(),
                    ANN_UNION_SEED_K,
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.topic_label_ann_union_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "topic_neighbor_ann_union_score",
                format!(
                    "{}_{}_precbp{}_q{}_k{}_c{}_ann{}_dim{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    neighbor_ann_union_precision,
                    fixture.query_count(),
                    TOP_K,
                    fixture.topic_neighbor_ann_union_count(),
                    ANN_UNION_SEED_K,
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.topic_neighbor_ann_union_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "topic_hint_expansion_score",
                format!(
                    "{}_{}_precbp{}_q{}_k{}_c{}_dim{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    hint_expansion_precision,
                    fixture.query_count(),
                    TOP_K,
                    fixture.topic_hint_expansion_count(),
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.topic_hint_expansion_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "topic_hint_expansion_cached_score",
                format!(
                    "{}_{}_precbp{}_q{}_k{}_c{}_dim{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    hint_expansion_cached_precision,
                    fixture.query_count(),
                    TOP_K,
                    fixture.topic_hint_expansion_cached_count(),
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.topic_hint_expansion_cached_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "topic_hint_expansion_bm25_score",
                format!(
                    "{}_{}_precbp{}_q{}_k{}_c{}_dim{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    hint_expansion_bm25_precision,
                    fixture.query_count(),
                    TOP_K,
                    fixture.topic_hint_expansion_bm25_count(),
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.topic_hint_expansion_bm25_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "topic_hint_expansion_bm25_vector_score",
                format!(
                    "{}_{}_precbp{}_q{}_k{}_c{}_dim{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    hint_expansion_bm25_vector_precision,
                    fixture.query_count(),
                    TOP_K,
                    fixture.topic_hint_expansion_bm25_vector_count(),
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.topic_hint_expansion_bm25_vector_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "topic_hint_expansion_state_score",
                format!(
                    "{}_{}_precbp{}_q{}_k{}_c{}_dim{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    hint_expansion_state_precision,
                    fixture.query_count(),
                    TOP_K,
                    fixture.topic_hint_expansion_state_count(),
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.topic_hint_expansion_state_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "ann_hint_expansion_state_score",
                format!(
                    "{}_{}_precbp{}_q{}_k{}_c{}_ann{}_dim{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    ann_hint_expansion_state_precision,
                    fixture.query_count(),
                    TOP_K,
                    fixture.ann_hint_expansion_state_count(),
                    ANN_UNION_SEED_K,
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.ann_hint_expansion_state_total_precision()));
            },
        );
        group.throughput(Throughput::Elements(mixed_cycle_elements as u64));
        group.bench_function(
            BenchmarkId::new(
                "topic_hint_expansion_cached_r60w40",
                format!(
                    "{}_{}_precbp{}_q{}_k{}_c{}_r{}w{}_totalc{}_dim{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    hint_expansion_cached_precision,
                    fixture.query_count(),
                    TOP_K,
                    fixture.topic_hint_expansion_cached_count(),
                    MIXED_READS_PER_CYCLE,
                    MIXED_REFRESHES_PER_CYCLE,
                    hint_expansion_refresh_candidates,
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.topic_hint_expansion_cached_mixed_read_refresh_work(
                        MIXED_ROUNDS_PER_CYCLE,
                        MIXED_READS_PER_ROUND,
                        MIXED_REFRESHES_PER_ROUND,
                    ));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "topic_hint_expansion_ann_union_score",
                format!(
                    "{}_{}_precbp{}_q{}_k{}_c{}_ann{}_dim{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    hint_expansion_ann_union_precision,
                    fixture.query_count(),
                    TOP_K,
                    fixture.topic_hint_expansion_ann_union_count(),
                    ANN_UNION_SEED_K,
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.topic_hint_expansion_ann_union_total_precision()));
            },
        );
        group.throughput(Throughput::Elements(fixture.query_count() as u64));
        group.bench_function(
            BenchmarkId::new(
                "topic_hint_expansion_refresh_sets",
                format!(
                    "{}_{}_q{}_c{}_totalc{}_dim{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    fixture.query_count(),
                    fixture.topic_hint_expansion_refresh_count(),
                    hint_expansion_refresh_candidates,
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.topic_hint_expansion_refresh_total_candidates()));
            },
        );
    }
    group.finish();
}

fn model_id(model: &str) -> String {
    model
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn append_target_hit(mut label: String, target_hit: Option<usize>) -> String {
    if let Some(target_hit) = target_hit {
        label.push_str(&format!("_hitbp{target_hit}"));
    }
    label
}

fn precision_basis_points(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(10_000)
        .checked_div(denominator)
        .unwrap_or(10_000)
}
