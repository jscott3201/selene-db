//! Local oMLX embedding support shared by benchmark binaries.
//!
//! This module is intentionally test-support only: it owns the deterministic
//! text corpora and the minimal HTTP client used by opt-in local embedding
//! benchmarks. CI compiles these paths but does not require a running oMLX
//! service.

mod client;
mod corpus;

pub use client::OmlxClient;
pub use corpus::{CorpusInput, CorpusProfile, Topic, topic_label};
