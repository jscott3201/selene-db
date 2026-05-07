#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! WAL entry header skeleton for `selene-persist`.
//!
//! D12 adds an opaque caller principal slot to the end of the v1 WAL header.
//! The field is postcard append-only: future header fields append after
//! `principal`, and older entries in the pre-shipping window decode as
//! `principal = None`. selene-db never interprets or redacts the bytes. See
//! `_spec/04-persistence-format.md` §3.2 for encoding and §3.6 for audit
//! iteration.

use std::sync::Arc;

/// Maximum principal byte length accepted in one WAL entry.
///
/// Exceeding this limit maps to GQLSTATUS `22023` at commit time. See
/// `_spec/04` §3.2 and D12.
pub const MAX_PRINCIPAL_BYTES: usize = 4096;

/// Logical v1 WAL entry header.
///
/// `principal` is intentionally the last field to preserve postcard append-only
/// discipline. Header filters in `wal_iterate` inspect this type before body
/// decoding. See `_spec/04` §3.6.
pub struct WalEntryHeader {
    /// Hybrid logical clock timestamp for the commit.
    pub hlc: Hlc,
    /// Origin of the mutation stream.
    pub origin: Origin,
    /// Coarse change kind for header-only filtering.
    pub kind: ChangeKind,
    /// Encoded WAL body length in bytes.
    pub body_len: u32,
    /// Opaque caller-owned principal bytes for D12 audit replay.
    pub principal: Option<Arc<[u8]>>,
}

impl WalEntryHeader {
    /// Construct a header after validating the principal-size cap.
    ///
    /// The final M3 implementation will also fill checksum, sequence, and flags
    /// fields from the physical WAL frame. See `_spec/04` §3.2.
    pub fn new(
        _hlc: Hlc,
        _origin: Origin,
        _kind: ChangeKind,
        _body_len: u32,
        _principal: Option<Arc<[u8]>>,
    ) -> Result<Self, WalHeaderError> {
        unimplemented!("M3 work")
    }
}

/// Errors raised while building or decoding WAL headers.
#[derive(Debug, thiserror::Error)]
pub enum WalHeaderError {
    /// Principal bytes exceeded [`MAX_PRINCIPAL_BYTES`].
    #[error("principal slot exceeds {MAX_PRINCIPAL_BYTES} bytes")]
    PrincipalTooLarge,
}

/// Placeholder HLC timestamp; the final type lives in `selene-core`.
pub struct Hlc {
    /// NTP64 seconds component.
    pub seconds: u64,
    /// Nanosecond-resolution subsecond component.
    pub subseconds: u32,
}

/// Placeholder mutation origin; the final type lives in `selene-core`.
pub enum Origin {
    /// Locally-authored mutation.
    Local,
    /// Replicated mutation from another graph node.
    Replicated {
        /// Source graph node identifier.
        source_node_id: u64,
        /// Source sequence number.
        source_seq: u64,
    },
}

/// Header-level change kind used for cheap WAL filtering.
pub enum ChangeKind {
    /// Node creation, update, or deletion.
    Node,
    /// Edge creation, update, or deletion.
    Edge,
    /// Schema or catalog mutation.
    Schema,
    /// Extension-owned mutation event.
    Extension,
}
