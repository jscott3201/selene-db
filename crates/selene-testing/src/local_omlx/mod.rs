//! Opt-in embedding support shared by benchmark binaries.
//!
//! This module is intentionally test-support only: it owns deterministic text
//! corpora, the minimal local oMLX HTTP client, and the curl-backed OpenRouter
//! client used by opt-in embedding benchmarks. CI compiles these paths but does
//! not require an embedding endpoint or API key.

mod client;
mod config;
mod corpus;

pub use client::{EmbeddingClient, EmbeddingProvider, OmlxClient, OpenRouterClient};
pub use config::EmbeddingBenchConfig;
pub use corpus::{CorpusInput, CorpusProfile, Topic, topic_label};
