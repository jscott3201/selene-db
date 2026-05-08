//! In-memory property graph runtime per spec 03.
//!
//! Storage, concurrency, built-in label indexes, and the typed mutation funnel
//! live here. Property indexes, schema validation for closed graphs, catalog
//! bootstrap, and persistence integration land in subsequent briefs.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod adjacency;
pub mod chunked_vec;
pub mod error;
pub mod graph;
pub mod id_allocator;
pub mod index_provider;
pub mod mutator;
pub(crate) mod reentry;
pub mod shared;
pub mod store;
pub mod write_txn;

pub use adjacency::{AdjacencyEdge, AdjacencyEntry};
pub use chunked_vec::ChunkedVec;
pub use error::{GraphError, GraphResult};
pub use graph::{GraphMeta, SeleneGraph};
pub use id_allocator::IdAllocator;
pub use index_provider::{IndexProvider, ProviderError, ProviderTag, SubTag};
pub use mutator::Mutator;
pub use shared::{SharedGraph, SharedGraphBuilder};
pub use store::{EdgeStore, NodeStore};
pub use write_txn::{CommitOutcome, WriteTxn};
