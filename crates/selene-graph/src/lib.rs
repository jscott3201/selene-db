//! In-memory property graph runtime per spec 03.
//!
//! Storage, concurrency, built-in label/property indexes, the typed mutation
//! funnel, and the auto-registered CORE persistence provider live here.
//! Composite indexes, edge property indexes, schema validation for closed
//! graphs, catalog bootstrap, and the procedure-pack `selene.create_index`
//! wrapper land in subsequent briefs.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod adjacency;
pub mod chunked_vec;
pub mod core_provider;
pub mod error;
pub mod graph;
pub mod graph_types;
pub mod id_allocator;
pub mod index_provider;
pub mod mutator;
pub(crate) mod property_index;
mod recover;
pub(crate) mod reentry;
pub mod shared;
pub mod store;
pub mod type_validator;
pub mod typed_index;
pub mod write_txn;

pub use adjacency::{AdjacencyEdge, AdjacencyEntry};
pub use chunked_vec::ChunkedVec;
pub use core_provider::{
    CORE_EDGE_SUB, CORE_GTYP_SUB, CORE_META_SUB, CORE_NODE_SUB, CORE_PROVIDER_TAG, CORE_SCMA_SUB,
    CoreProvider,
};
pub use error::{GraphError, GraphResult};
pub use graph::{GraphMeta, SeleneGraph};
pub use graph_types::{EdgeTypeDef, GraphTypeDef, NodeTypeDef, PropertyTypeDef};
pub use id_allocator::IdAllocator;
pub use index_provider::{IndexProvider, ProviderError, ProviderTag, SubTag};
pub use mutator::Mutator;
pub use shared::{SharedGraph, SharedGraphBuilder};
pub use store::{EdgeStore, NodeStore};
pub use type_validator::{EntityId, TypeViolation, validate_change, validate_entity_state};
pub use typed_index::{NotNanError, NotNanF64, TypedIndex, TypedIndexKind};
pub use write_txn::{CommitOutcome, WriteTxn};

#[cfg(test)]
mod closed_graph_tests;
