//! In-memory property graph runtime per spec 03.
//!
//! Storage, concurrency, and the typed mutation funnel live here. Indexes
//! (spec 03 section 5), schema validation for closed graphs, and persistence
//! integration land in subsequent briefs.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod adjacency;
pub mod chunked_vec;
pub mod error;
pub mod graph;
pub mod id_allocator;
pub mod mutator;
pub mod shared;
pub mod store;
pub mod write_txn;

pub use adjacency::{AdjacencyEdge, AdjacencyEntry};
pub use chunked_vec::ChunkedVec;
pub use error::{GraphError, GraphResult};
pub use graph::{GraphMeta, SeleneGraph};
pub use id_allocator::IdAllocator;
pub use mutator::Mutator;
pub use shared::SharedGraph;
pub use store::{EdgeStore, NodeStore};
pub use write_txn::{CommitOutcome, WriteTxn};
