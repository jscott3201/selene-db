#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Snapshot envelope and section-table skeleton for `selene-persist`.
//!
//! BRIEF-04a / D13 locks the snapshot section table at 24-byte rows:
//! `ProviderTag` (4 bytes), `SubTag` (4 bytes), `offset` (8 bytes), and
//! `length` (8 bytes). BRIEF-04a / D14 records that M3 will derive or otherwise
//! use `rkyv::Archive` over sorted-vec intermediate payloads, then materialize
//! imbl collections via `from_iter` on cold start. See `_spec/04` §4.2-§4.3.

use std::path::Path;

/// Snapshot file magic.
pub const SNAPSHOT_MAGIC: &[u8; 4] = b"SLSN";

/// Snapshot format version for v1.0.
pub const SNAPSHOT_VERSION: u16 = 1;

/// Number of bytes in one snapshot section table row.
pub const SECTION_TABLE_ROW_BYTES: usize = 24;

/// Snapshot file envelope.
///
/// Payload sections follow at the offsets named by [`SectionTable`]. The final
/// M3 type will archive payloads using rkyv-friendly sorted-vec intermediates
/// per D14.
pub struct SnapshotEnvelope {
    /// Magic bytes, always [`SNAPSHOT_MAGIC`].
    pub magic: [u8; 4],
    /// Snapshot format version, always [`SNAPSHOT_VERSION`] for v1.
    pub version: u16,
    /// Compression and layout flags.
    pub flags: u16,
    /// WAL sequence covered by this snapshot.
    pub sequence: u64,
    /// Hybrid logical clock timestamp for snapshot publication.
    pub hlc: Hlc,
    /// Snapshot section table.
    pub section_table: SectionTable,
}

impl SnapshotEnvelope {
    /// Decode a snapshot envelope from disk.
    ///
    /// M3 will validate magic, version, flags, body hash, and the 24-byte row
    /// table described in spec 04 §4.2.
    pub fn read_from(_path: &Path) -> Result<Self, SnapshotError> {
        unimplemented!("M3 work")
    }

    /// Encode a snapshot envelope and payload sections to disk.
    ///
    /// M3 will write through a temporary file and atomic rename per spec 04
    /// §4.4.
    pub fn write_to(&self, _path: &Path) -> Result<(), SnapshotError> {
        unimplemented!("M3 work")
    }
}

/// Snapshot section table.
pub struct SectionTable {
    /// Rows sorted by file offset.
    pub rows: Vec<SectionTableRow>,
}

/// One 24-byte section table row.
pub struct SectionTableRow {
    /// Provider tag, four bytes.
    pub provider_tag: ProviderTag,
    /// Provider-owned sub-tag, four bytes.
    pub sub_tag: SubTag,
    /// File-absolute payload offset, eight bytes.
    pub offset: u64,
    /// Payload byte length, eight bytes.
    pub length: u64,
}

/// Stable 4-byte ASCII provider identifier.
pub struct ProviderTag(
    /// Provider tag bytes.
    pub [u8; 4],
);

/// Provider-owned 4-byte snapshot sub-section identifier.
pub struct SubTag(
    /// Sub-tag bytes.
    pub [u8; 4],
);

/// Placeholder HLC timestamp; the final type lives in `selene-core`.
pub struct Hlc {
    /// NTP64 seconds component.
    pub seconds: u64,
    /// Nanosecond-resolution subsecond component.
    pub subseconds: u32,
}

/// Snapshot envelope error placeholder.
#[derive(Debug)]
pub enum SnapshotError {
    /// Placeholder until M3 defines concrete decode and encode failures.
    M3Placeholder,
}
