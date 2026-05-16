//! HNSW graph model and distance kernels.
//!
//! HNSW stores one vector node with a neighbor list for every layer from layer
//! 0 through `max_layer`, inclusive. Layer 0 is the dense base graph; higher
//! layers are progressively sparser routing layers, layer sampling follows the
//! standard exponential-decay formula, and `MAX_LAYER` caps persisted layer
//! width. The provider owns mutation replay and snapshot section routing; this
//! module owns graph shape, insertion/search invariants, and metric scoring.
//! See Malkov and Yashunin 2018 and Spec 17 VECT/GRPH/VECS invariants.

pub mod build;
pub(crate) mod delete;
pub mod distance;
mod graph;
pub mod params;
pub mod search;

pub use build::{insert_node, random_layer, random_layer_default};
pub use graph::{HnswGraph, HnswNode};
pub use params::HnswParams;

/// Provider-local HNSW row index.
///
/// This is distinct from graph `NodeId`: it addresses the vector index's
/// compact `Vec<HnswNode>` storage.
pub type InternalIndex = u32;
