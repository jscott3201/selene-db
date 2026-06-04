//! Local-only oMLX embedding rows for realistic vector distributions.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};

use crate::common::scale_label;

use self::{client::OmlxClient, corpus::CorpusProfile, fixture::OmlxVectorFixture};

mod client;
mod corpus;
mod fixture;

const ENABLE_ENV: &str = "SELENE_OMLX_EMBEDDING_BENCH";
const API_KEY_ENVS: &[&str] = &["SELENE_OMLX_API_KEY", "OMLX_KEY"];
const BASE_URL_ENV: &str = "SELENE_OMLX_BASE_URL";
const MODELS_ENV: &str = "SELENE_OMLX_EMBEDDING_MODELS";
const CORPUS_ENV: &str = "SELENE_OMLX_CORPUS";
const BATCH_SIZE_ENV: &str = "SELENE_OMLX_EMBEDDING_BATCH_SIZE";
const GRAPH_HINT_DOCS_PER_TOPIC_ENV: &str = "SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC";
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:7700/v1";
const DEFAULT_MODELS: &[&str] = &[
    "Qwen3-Embedding-0.6B-4bit-DWQ",
    "Qwen3-Embedding-4B-4bit-DWQ",
];
const DEFAULT_EMBEDDING_BATCH_SIZE: usize = 64;
const TOP_K: usize = 4;
const ANN_SEARCH_WIDTH: usize = 64;
const ANN_UNION_SEED_K: usize = 8;

pub(super) fn bench(c: &mut Criterion) {
    let Some(config) = OmlxBenchConfig::from_env() else {
        return;
    };
    let client = OmlxClient::new(config.base_url, config.api_key, config.batch_size);
    let inputs = config.corpus.inputs();
    let mut group = c.benchmark_group("graph_vector_omlx_embedding_pressure");
    for model in config.models {
        let model_id = model_id(&model);
        let vectors = client
            .embed(&model, &inputs)
            .expect("local oMLX embedding request succeeds");
        let fixture =
            OmlxVectorFixture::build(&model, &inputs, vectors, config.graph_hint_docs_per_topic);
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
        let hint_expansion_ann_union_precision = precision_basis_points(
            fixture.topic_hint_expansion_ann_union_total_precision(),
            fixture.query_count() * TOP_K,
        );
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
                        client
                            .embed(&model, &inputs)
                            .expect("local oMLX embedding request succeeds"),
                    );
                });
            },
        );
        group.throughput(Throughput::Elements((fixture.query_count() * TOP_K) as u64));
        group.bench_function(
            BenchmarkId::new(
                "exact_graph_search",
                format!(
                    "{}_{}_q{}_k{}_dim{}_precbp{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    fixture.query_count(),
                    TOP_K,
                    fixture.dimension,
                    fixture.exact_precision_basis_points(),
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.exact_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "hnsw_graph_search",
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
            ),
            |b| {
                b.iter(|| black_box(fixture.ann_total_precision()));
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
    }
    group.finish();
}

struct OmlxBenchConfig {
    base_url: String,
    api_key: String,
    models: Vec<String>,
    corpus: CorpusProfile,
    batch_size: usize,
    graph_hint_docs_per_topic: Option<usize>,
}

impl OmlxBenchConfig {
    fn from_env() -> Option<Self> {
        if std::env::var(ENABLE_ENV).ok().as_deref() != Some("1") {
            return None;
        }
        let api_key = API_KEY_ENVS
            .iter()
            .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
            .expect("SELENE_OMLX_API_KEY or OMLX_KEY must be set for local oMLX benches");
        let base_url = std::env::var(BASE_URL_ENV).unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned());
        let models = std::env::var(MODELS_ENV)
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|models| !models.is_empty())
            .unwrap_or_else(|| {
                DEFAULT_MODELS
                    .iter()
                    .map(|model| (*model).to_owned())
                    .collect()
            });
        Some(Self {
            base_url,
            api_key,
            models,
            corpus: CorpusProfile::from_env(CORPUS_ENV),
            batch_size: embedding_batch_size(),
            graph_hint_docs_per_topic: graph_hint_docs_per_topic(),
        })
    }
}

fn embedding_batch_size() -> usize {
    std::env::var(BATCH_SIZE_ENV)
        .ok()
        .map(|raw| {
            let batch_size = raw
                .parse::<usize>()
                .expect("SELENE_OMLX_EMBEDDING_BATCH_SIZE must be a positive integer");
            assert!(
                batch_size > 0,
                "SELENE_OMLX_EMBEDDING_BATCH_SIZE must be greater than zero"
            );
            batch_size
        })
        .unwrap_or(DEFAULT_EMBEDDING_BATCH_SIZE)
}

fn graph_hint_docs_per_topic() -> Option<usize> {
    std::env::var(GRAPH_HINT_DOCS_PER_TOPIC_ENV)
        .ok()
        .filter(|raw| !raw.is_empty())
        .map(|raw| {
            raw.parse::<usize>()
                .expect("SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC must be a non-negative integer")
        })
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

fn precision_basis_points(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(10_000)
        .checked_div(denominator)
        .unwrap_or(10_000)
}
