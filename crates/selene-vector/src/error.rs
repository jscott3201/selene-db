//! Error types for the selene-vector extension.

use selene_graph::SubTag;

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
}
