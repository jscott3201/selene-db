use std::{hint::black_box, num::NonZeroUsize, sync::Arc};

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_gql::{BuiltinProcedureRegistry, CallPlanCache};
use selene_testing::local_omlx::{CorpusProfile, OmlxClient};

#[path = "vector_omlx_query_roots/fixture.rs"]
mod fixture;

use fixture::{OmlxGqlQueryRootFixture, TOP_K};

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

pub(super) fn bench_vector_omlx_query_roots_procedure(c: &mut Criterion) {
    let Some(config) = OmlxBenchConfig::from_env() else {
        return;
    };
    let client = OmlxClient::new(config.base_url, config.api_key, config.batch_size);
    let inputs = config.corpus.inputs();
    let registry = BuiltinProcedureRegistry::new();
    let mut group = c.benchmark_group("procedure_vector_omlx_query_roots");
    for model in config.models {
        let model_id = model_id(&model);
        let vectors = client
            .embed(&model, &inputs)
            .expect("local oMLX embedding request succeeds");
        let fixture = OmlxGqlQueryRootFixture::build(
            &model,
            &inputs,
            vectors,
            config.graph_hint_docs_per_topic,
        );
        let cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        let state_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        let batch_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        fixture.warm_query_root_cache(&registry, Arc::clone(&cache));
        fixture.warm_query_root_state_cache(&registry, Arc::clone(&state_cache));
        fixture.warm_query_root_batch_cache(&registry, Arc::clone(&batch_cache));
        let precision = fixture.gql_precision_basis_points(&registry, Some(Arc::clone(&cache)));
        let state_precision =
            fixture.gql_state_precision_basis_points(&registry, Some(Arc::clone(&state_cache)));
        let batch_precision =
            fixture.gql_batch_precision_basis_points(&registry, Some(Arc::clone(&batch_cache)));
        group.throughput(Throughput::Elements((fixture.query_count() * TOP_K) as u64));
        group.bench_function(
            BenchmarkId::new(
                "shared_cache_query_root_expansion",
                format!(
                    "{}_{}_q{}_k{}_r{}_c{}_dim{}_precbp{}",
                    model_id,
                    corpus_label(config.corpus),
                    fixture.query_count(),
                    TOP_K,
                    fixture.first_query_root_count(),
                    fixture.first_query_expanded_count(),
                    fixture.dimension,
                    precision,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_all_queries(&registry, Some(Arc::clone(&cache))));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_cache_query_root_state_intersection",
                format!(
                    "{}_{}_q{}_k{}_r{}_c{}_dim{}_precbp{}",
                    model_id,
                    corpus_label(config.corpus),
                    fixture.query_count(),
                    TOP_K,
                    fixture.first_query_root_count(),
                    fixture.first_query_state_intersection_count(),
                    fixture.dimension,
                    state_precision,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(
                        fixture
                            .execute_all_state_queries(&registry, Some(Arc::clone(&state_cache))),
                    );
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_cache_query_root_expansion_batch",
                format!(
                    "{}_{}_q{}_k{}_r{}_c{}_dim{}_precbp{}",
                    model_id,
                    corpus_label(config.corpus),
                    fixture.query_count(),
                    TOP_K,
                    fixture.first_query_root_count(),
                    fixture.first_query_expanded_count(),
                    fixture.dimension,
                    batch_precision,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(
                        fixture.execute_batch_query(&registry, Some(Arc::clone(&batch_cache))),
                    );
                });
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

fn corpus_label(corpus: CorpusProfile) -> &'static str {
    match corpus {
        CorpusProfile::Tiny => "tiny",
        CorpusProfile::AgentMemory => "agent_memory",
        CorpusProfile::AmbiguousMemory => "ambiguous_memory",
        CorpusProfile::ScaledAmbiguousMemory => "scaled_ambiguous_memory",
    }
}
