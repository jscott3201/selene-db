//! Opt-in HNSW vector index extension for selene-db.
//!
//! `selene-vector` is the first extension crate outside the v1.0 mandatory
//! crate set. It owns the `VECT` [`selene_graph::IndexProvider`] registration
//! and the future `vector.*` procedure-pack namespace.
//!
//! BRIEF-60 adds the read-only HNSW graph shape, scalar distance kernels,
//! fresh-vector insertion, replay of `IndexExtensionEvent` payloads, and HNSW
//! search with an optional RoaringBitmap pre-filter:
//!
//! ```
//! use selene_vector::distance::{cosine_similarity, dot_product, l2_squared};
//!
//! let a = [1.0f32, 0.0, 0.0, 0.0];
//! let b = [0.0f32, 1.0, 0.0, 0.0];
//! assert_eq!(dot_product(&a, &b), 0.0);
//! assert_eq!(l2_squared(&a, &b), 2.0);
//! assert_eq!(cosine_similarity(&a, &b), 0.0);
//! ```
//!
//! BRIEF-61 adds deterministic GRPH/VECS snapshot bodies. Procedure
//! registration, quantization, and the D21 snapshot harness land in later M8
//! briefs.
//!
//! The Rust crate name is `selene-vector`, while the procedure-pack name is
//! `vector`. Future procedures therefore register as `vector.knn`,
//! `vector.cosine_sim`, and `vector.upsert`.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub(crate) mod builder;
pub mod config;
pub mod error;
pub mod hnsw;
pub mod payload;
pub mod procedures;
pub mod provider;
pub(crate) mod snapshot;

pub use config::{DistanceMetric, HnswConfig};
pub use error::VectorError;
pub use hnsw::distance;
pub use hnsw::{HnswGraph, HnswNode, HnswParams, insert_node, random_layer, random_layer_default};
pub use payload::{PAYLOAD_MAGIC, VectorOp, VectorUpsertPayloadV1};
pub use procedures::pack_manifest;
pub use provider::HnswProvider;
