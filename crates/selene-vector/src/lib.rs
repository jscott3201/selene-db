//! Opt-in HNSW vector index extension for selene-db.
//!
//! `selene-vector` is the first extension crate outside the v1.0 mandatory
//! crate set. It owns the `VECT` [`selene_graph::IndexProvider`] registration
//! and the future `vector.*` procedure-pack namespace.
//!
//! BRIEF-57 intentionally ships only the provider skeleton. HNSW graph
//! construction, search, snapshot bodies, procedure registration, quantization,
//! and the D21 snapshot harness land in later M8 briefs.
//!
//! The Rust crate name is `selene-vector`, while the procedure-pack name is
//! `vector`. Future procedures therefore register as `vector.knn`,
//! `vector.cosine_sim`, and `vector.upsert`.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod config;
pub mod error;
pub mod procedures;
pub mod provider;
pub(crate) mod snapshot;

pub use config::{DistanceMetric, HnswConfig};
pub use error::VectorError;
pub use procedures::pack_manifest;
pub use provider::HnswProvider;
