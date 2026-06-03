//! In-memory property graph runtime.
//!
//! The graph crate owns node/edge storage, label sets, property maps, directed
//! adjacency, built-in label/property indexes, typed mutation validation, and
//! the CORE persistence provider. `SharedGraph` serializes writes through a
//! transaction boundary while readers observe immutable snapshots. selene-db is
//! a single native engine: the higher `selene-gql` layer owns GQL
//! binding/planning and the one frozen native procedure registry. Edge property
//! indexes remain outside the v1.0 storage contract.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod adjacency;
pub mod chunked_vec;
pub(crate) mod committer;
pub(crate) mod committer_batch;
pub mod compaction;
pub(crate) mod composite_property_index;
pub mod composite_typed_index;
mod consistency;
pub mod core_provider;
pub mod durable_provider;
pub mod error;
pub mod graph;
pub mod graph_types;
pub mod id_allocator;
pub mod index_provider;
pub mod mutator;
pub(crate) mod panic_payload;
pub(crate) mod property_index;
mod recover;
pub(crate) mod reentry;
pub mod shared;
pub mod store;
pub mod type_validator;
pub mod typed_index;
pub mod vector_index;
pub mod vector_search;
pub mod write_txn;

pub use adjacency::{AdjacencyEdge, AdjacencyEntry};
pub use chunked_vec::ChunkedVec;
pub use committer_batch::CommitBatching;
pub use compaction::{CompactedCore, CompactionReport, compact_core};
pub use composite_typed_index::{
    CompositeIndexValueError, CompositeKey, CompositeKeyComponent, CompositeTypedIndex,
};
pub use core_provider::{
    CORE_EDGE_SUB, CORE_GTYP_SUB, CORE_META_SUB, CORE_NODE_SUB, CORE_PROVIDER_TAG, CORE_SCMA_SUB,
    CORE_VIDX_SUB, CoreProvider, DurableState,
};
pub use durable_provider::DurableProvider;
pub use error::{GraphError, GraphResult};
pub use graph::{CompositePropertyIndexEntry, GraphMeta, SeleneGraph, VectorIndexEntry};
pub use graph_types::{
    DropBehavior, EdgeEndpointDef, EdgeTypeDef, GraphTypeDef, MAX_RECORD_TYPE_NESTING, NodeTypeDef,
    PropertyDefaultValue, PropertyElementType, PropertyTypeDef, RecordFieldType,
    RecordFieldTypeDef, RecordFieldTypes, ValidationMode,
};
pub use id_allocator::IdAllocator;
pub use index_provider::{IndexProvider, ProviderError, ProviderTag, SubTag};
pub use mutator::Mutator;
pub use selene_persist::{DEFAULT_WAL_FILE_NAME, SyncPolicy, WalConfig};
pub use shared::{SharedGraph, SharedGraphBuilder};
pub use store::{EdgeStore, NodeStore, RowIndex};
pub use type_validator::{EntityId, TypeViolation, validate_change, validate_entity_state};
pub use typed_index::{NotNanError, NotNanF64, TypedIndex, TypedIndexKind};
pub use vector_index::{VectorIndex, VectorIndexKind, VectorIndexValueError};
pub use vector_search::{ApproximateVectorSearchOptions, VectorNodeSearchHit, VectorSearchError};
pub use write_txn::{CommitOutcome, CommitWarning, WriteTxn};

#[cfg(test)]
mod closed_graph_tests;
