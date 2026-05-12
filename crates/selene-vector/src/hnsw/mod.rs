//! Read-only HNSW graph model and distance kernels.
//!
//! HNSW stores one vector node with a neighbor list for every layer from
//! layer 0 through `max_layer`, inclusive. Layer 0 is the dense base graph;
//! higher layers are progressively sparser routing layers. BRIEF-58 freezes
//! this read surface without adding insertion, search, or snapshot codecs.

pub mod distance;
mod graph;

pub use graph::{HnswGraph, HnswNode};

/// Provider-local HNSW row index.
///
/// This is distinct from graph `NodeId`: it addresses the vector index's
/// compact `Vec<HnswNode>` storage.
pub type InternalIndex = u32;
