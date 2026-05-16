//! Opt-in HNSW vector index extension for selene-db.
//!
//! `selene-vector` is the first extension crate outside the v1.0 mandatory
//! crate set. It owns the `VECT` [`selene_graph::IndexProvider`] registration
//! and the future `vector.*` procedure-pack namespace.
//!
//! The crate provides the read-only HNSW graph shape, scalar distance kernels,
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
//! Deterministic GRPH/VECS snapshot bodies, the VECB bulk-insert wire format,
//! QUNT quantization overlays, and asymmetric search all live here. Procedure
//! registration moves to future `selene-vector-pack` work.
//!
//! The Rust crate name is `selene-vector`, while the procedure-pack name is
//! `vector`. Future procedures therefore register as `vector.knn`,
//! `vector.cosine_sim`, and `vector.upsert`.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub(crate) mod builder;
pub(crate) mod clustering;
pub mod config;
pub mod error;
pub mod hnsw;
pub mod ivf;
pub mod payload;
pub mod procedures;
pub mod provider;
pub(crate) mod quantize;
pub mod registry;
pub(crate) mod snapshot;
#[cfg(any(test, feature = "test-harness"))]
pub mod snapshot_summary;

pub use config::{DistanceMetric, HnswConfig, IvfConfig, NeighborSelectionConfig};
pub use error::VectorError;
pub use hnsw::distance;
pub use hnsw::{HnswGraph, HnswNode, HnswParams, insert_node, random_layer, random_layer_default};
pub use ivf::{IvfIndex, IvfProvider, IvfStats};
pub use payload::{
    BulkInsertRow, IvfBulkInsertRow, PAYLOAD_MAGIC, PAYLOAD_MAGIC_BULK, PAYLOAD_MAGIC_BULK_DELETE,
    PAYLOAD_MAGIC_IVF, PAYLOAD_MAGIC_IVF_BULK_DELETE, PAYLOAD_MAGIC_IVF_BULK_INSERT,
    VectorBulkDeletePayloadV1, VectorBulkInsertPayloadV1, VectorIvfBulkDeleteV1,
    VectorIvfBulkInsertV1, VectorIvfUpsertV1, VectorOp, VectorUpsertPayloadV1,
};
pub use procedures::pack_manifest;
pub use provider::HnswProvider;
pub use quantize::{
    PqParams, QuantMethod, QuantizationConfig, QuantizationStats, QuantizationStatsKind,
    opq_max_dim,
};
pub use registry::{Catalog, HnswIndexRegistry, IvfIndexRegistry};
#[cfg(any(test, feature = "test-harness"))]
pub use snapshot_summary::{VectorInvocationResult, VectorSnapshot, VectorSnapshotInput};
