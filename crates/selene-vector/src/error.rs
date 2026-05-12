//! Error types for the selene-vector extension.

use selene_core::NodeId;
use selene_graph::SubTag;

use crate::VectorOp;

/// Errors returned by selene-vector public APIs.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum VectorError {
    /// HNSW configuration failed validation.
    #[error("invalid HNSW configuration: {reason}")]
    #[diagnostic(code(SLENE_VEC_001))]
    InvalidConfig {
        /// Human-readable validation failure.
        reason: String,
    },

    /// A supplied vector did not match the provider's configured dimension.
    #[error("vector dimension mismatch: expected {expected}, observed {observed}")]
    #[diagnostic(code(SLENE_VEC_002))]
    DimensionMismatch {
        /// Expected vector dimension.
        expected: usize,
        /// Observed vector dimension.
        observed: usize,
    },

    /// A provider snapshot section could not be decoded.
    #[error("failed to decode vector section {sub_tag}: {reason}")]
    #[diagnostic(code(SLENE_VEC_003))]
    SectionDecodeFailed {
        /// Provider-local subsection tag.
        sub_tag: SubTag,
        /// Human-readable decode failure.
        reason: String,
    },

    /// A provider snapshot section could not be encoded.
    #[error("failed to encode vector section {sub_tag}: {reason}")]
    #[diagnostic(code(SLENE_VEC_004))]
    SectionEncodeFailed {
        /// Provider-local subsection tag.
        sub_tag: SubTag,
        /// Human-readable encode failure.
        reason: String,
    },

    /// A source graph node ID is not valid for HNSW indexing.
    #[error("invalid node id {node_id}: {reason}")]
    #[diagnostic(code(SLENE_VEC_005))]
    InvalidNodeId {
        /// Rejected source graph node ID.
        node_id: NodeId,
        /// Human-readable validation failure.
        reason: String,
    },

    /// A mutation payload vector disagrees with the provider's locked dimension.
    #[error("vector dimensions locked: expected {expected}, observed {observed}")]
    #[diagnostic(code(SLENE_VEC_006))]
    DimensionsLocked {
        /// Expected provider dimension.
        expected: usize,
        /// Observed payload dimension.
        observed: usize,
    },

    /// A selene-vector mutation event payload could not be decoded.
    #[error("invalid selene-vector payload: {reason}")]
    #[diagnostic(code(SLENE_VEC_007))]
    InvalidPayload {
        /// Human-readable decode or validation failure.
        reason: String,
    },

    /// A selene-vector mutation event payload could not be encoded.
    #[error("failed to encode selene-vector payload: {reason}")]
    #[diagnostic(code(SLENE_VEC_008))]
    EncodeFailed {
        /// Human-readable encode failure.
        reason: String,
    },

    /// A vector mutation op is reserved but not yet implemented.
    #[error("vector op {op:?} on node id {node_id} not yet supported; see {brief}")]
    #[diagnostic(code(SLENE_VEC_009))]
    OperationNotSupportedYet {
        /// Unsupported operation.
        op: VectorOp,
        /// Source graph node ID.
        node_id: NodeId,
        /// Brief expected to land support.
        brief: &'static str,
    },

    /// A fresh insert attempted to reuse an already-indexed node ID.
    #[error("duplicate vector node id {node_id}")]
    #[diagnostic(code(SLENE_VEC_010))]
    DuplicateNodeId {
        /// Duplicated source graph node ID.
        node_id: NodeId,
    },

    /// A vector component was NaN or infinite.
    #[error("non-finite vector component for node id {node_id} at index {index}: {value}")]
    #[diagnostic(code(SLENE_VEC_011))]
    NonFiniteVectorComponent {
        /// Source graph node ID.
        node_id: NodeId,
        /// Component index.
        index: usize,
        /// Non-finite component value.
        value: f32,
    },

    /// HNSW row index space is exhausted (`InternalIndex = u32`).
    #[error("HNSW row index exhausted: node count {current} reached the u32 cap")]
    #[diagnostic(code(SLENE_VEC_012))]
    InternalIndexExhausted {
        /// Node count when exhaustion was detected.
        current: usize,
    },

    /// A direct HNSW insertion requested a layer above the per-spec cap.
    #[error("HNSW max_layer {observed} exceeds cap {cap}")]
    #[diagnostic(code(SLENE_VEC_013))]
    MaxLayerExceedsCap {
        /// Rejected `max_layer` value.
        observed: u8,
        /// Per-spec cap (also enforced by payload decoder).
        cap: u8,
    },
}
