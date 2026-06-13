//! Shared opt-in embedding benchmark configuration.

use super::client::{EmbeddingClient, EmbeddingProvider};
use super::corpus::{CorpusInput, CorpusProfile, scale_document_inputs};
use selene_core::VectorValue;

const ENABLE_ENV: &str = "SELENE_EMBEDDING_BENCH";
const LEGACY_ENABLE_ENV: &str = "SELENE_OMLX_EMBEDDING_BENCH";
const ENABLE_ENVS: &[&str] = &[ENABLE_ENV, LEGACY_ENABLE_ENV];
const PROVIDER_ENV: &str = "SELENE_EMBEDDING_PROVIDER";
const MODELS_ENVS: &[&str] = &["SELENE_EMBEDDING_MODELS", "SELENE_OMLX_EMBEDDING_MODELS"];
const BATCH_SIZE_ENVS: &[&str] = &[
    "SELENE_EMBEDDING_BATCH_SIZE",
    "SELENE_OMLX_EMBEDDING_BATCH_SIZE",
];
const CORPUS_ENVS: &[&str] = &["SELENE_EMBEDDING_CORPUS", "SELENE_OMLX_CORPUS"];
const CORPUS_REPEAT_ENVS: &[&str] = &[
    "SELENE_EMBEDDING_CORPUS_REPEAT",
    "SELENE_OMLX_CORPUS_REPEAT",
];
const GRAPH_HINT_DOCS_PER_TOPIC_ENVS: &[&str] = &[
    "SELENE_GRAPH_HINT_DOCS_PER_TOPIC",
    "SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC",
];
const OMLX_API_KEY_ENVS: &[&str] = &["SELENE_OMLX_API_KEY", "OMLX_KEY"];
const OPENROUTER_API_KEY_ENVS: &[&str] = &["SELENE_OPENROUTER_API_KEY", "OPENROUTER_API_KEY"];
const OMLX_BASE_URL_ENV: &str = "SELENE_OMLX_BASE_URL";
const OPENROUTER_BASE_URL_ENV: &str = "SELENE_OPENROUTER_BASE_URL";
const OPENROUTER_REFERER_ENV: &str = "SELENE_OPENROUTER_HTTP_REFERER";
const OPENROUTER_TITLE_ENV: &str = "SELENE_OPENROUTER_TITLE";
const DEFAULT_OMLX_BASE_URL: &str = "http://127.0.0.1:7700/v1";
const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_OMLX_MODELS: &[&str] = &[
    "Qwen3-Embedding-0.6B-4bit-DWQ",
    "Qwen3-Embedding-4B-4bit-DWQ",
];
const DEFAULT_OPENROUTER_MODELS: &[&str] = &["mistralai/codestral-embed-2505"];
const DEFAULT_EMBEDDING_BATCH_SIZE: usize = 64;
const DEFAULT_EMBEDDING_CORPUS_REPEAT: usize = 1;

/// Shared configuration for local/remote opt-in embedding benchmark rows.
pub struct EmbeddingBenchConfig {
    /// Embedding client provider.
    pub provider: EmbeddingProvider,
    /// Model identifiers to request from the provider.
    pub models: Vec<String>,
    /// Corpus profile to embed.
    pub corpus: CorpusProfile,
    /// Number of times to repeat document inputs before keeping one query set.
    pub corpus_repeat: usize,
    /// Request chunk size.
    pub batch_size: usize,
    /// Optional number of graph hint docs per topic.
    pub graph_hint_docs_per_topic: Option<usize>,
    client: EmbeddingClient,
}

impl EmbeddingBenchConfig {
    /// Read benchmark configuration from environment variables.
    ///
    /// Returns `None` unless either `SELENE_EMBEDDING_BENCH=1` or the legacy
    /// `SELENE_OMLX_EMBEDDING_BENCH=1` is set.
    pub fn from_env() -> Option<Self> {
        if !enabled() {
            return None;
        }
        let provider = EmbeddingProvider::from_env(PROVIDER_ENV, default_provider());
        let batch_size = embedding_batch_size();
        let client = match provider {
            EmbeddingProvider::Omlx => {
                let api_key = required_env(OMLX_API_KEY_ENVS, "SELENE_OMLX_API_KEY or OMLX_KEY");
                let base_url = std::env::var(OMLX_BASE_URL_ENV)
                    .unwrap_or_else(|_| DEFAULT_OMLX_BASE_URL.to_owned());
                EmbeddingClient::omlx(base_url, api_key, batch_size)
            }
            EmbeddingProvider::OpenRouter => {
                let api_key = required_env(
                    OPENROUTER_API_KEY_ENVS,
                    "SELENE_OPENROUTER_API_KEY or OPENROUTER_API_KEY",
                );
                let base_url = std::env::var(OPENROUTER_BASE_URL_ENV)
                    .unwrap_or_else(|_| DEFAULT_OPENROUTER_BASE_URL.to_owned());
                let referer = optional_env(OPENROUTER_REFERER_ENV);
                let title = optional_env(OPENROUTER_TITLE_ENV);
                EmbeddingClient::openrouter(base_url, api_key, batch_size, referer, title)
            }
        };
        Some(Self {
            provider,
            models: models(provider),
            corpus: corpus_profile(),
            corpus_repeat: corpus_repeat(),
            batch_size,
            graph_hint_docs_per_topic: graph_hint_docs_per_topic(),
            client,
        })
    }

    /// Materialize configured corpus inputs, including benchmark-only scaling.
    pub fn inputs(&self) -> Vec<CorpusInput> {
        scale_document_inputs(self.corpus.inputs(), self.corpus_repeat)
    }

    /// Embed every corpus input with `model`.
    pub fn embed(&self, model: &str, inputs: &[CorpusInput]) -> Result<Vec<VectorValue>, String> {
        self.client.embed(model, inputs)
    }
}

fn enabled() -> bool {
    ENABLE_ENVS
        .iter()
        .any(|name| std::env::var(name).ok().as_deref() == Some("1"))
}

fn default_provider() -> EmbeddingProvider {
    if std::env::var(ENABLE_ENV).ok().as_deref() == Some("1") {
        EmbeddingProvider::OpenRouter
    } else {
        EmbeddingProvider::Omlx
    }
}

fn models(provider: EmbeddingProvider) -> Vec<String> {
    env_from_any(MODELS_ENVS)
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|models| !models.is_empty())
        .unwrap_or_else(|| {
            let defaults = match provider {
                EmbeddingProvider::Omlx => DEFAULT_OMLX_MODELS,
                EmbeddingProvider::OpenRouter => DEFAULT_OPENROUTER_MODELS,
            };
            defaults.iter().map(|model| (*model).to_owned()).collect()
        })
}

fn embedding_batch_size() -> usize {
    env_from_any(BATCH_SIZE_ENVS)
        .map(|raw| {
            let batch_size = raw
                .parse::<usize>()
                .expect("embedding batch size must be a positive integer");
            assert!(
                batch_size > 0,
                "embedding batch size must be greater than zero"
            );
            batch_size
        })
        .unwrap_or(DEFAULT_EMBEDDING_BATCH_SIZE)
}

fn corpus_profile() -> CorpusProfile {
    env_from_any(CORPUS_ENVS)
        .as_deref()
        .map_or(CorpusProfile::Tiny, CorpusProfile::from_value)
}

fn corpus_repeat() -> usize {
    env_from_any(CORPUS_REPEAT_ENVS)
        .map(|raw| {
            let repeat = raw
                .parse::<usize>()
                .expect("embedding corpus repeat must be a positive integer");
            assert!(
                repeat > 0,
                "embedding corpus repeat must be greater than zero"
            );
            repeat
        })
        .unwrap_or(DEFAULT_EMBEDDING_CORPUS_REPEAT)
}

fn graph_hint_docs_per_topic() -> Option<usize> {
    env_from_any(GRAPH_HINT_DOCS_PER_TOPIC_ENVS).map(|raw| {
        raw.parse::<usize>()
            .expect("graph hint docs per topic must be a non-negative integer")
    })
}

fn required_env(names: &[&str], description: &str) -> String {
    env_from_any(names).unwrap_or_else(|| panic!("{description} must be set for embedding benches"))
}

fn optional_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn env_from_any(names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
}
