//! Fixed snapshot file header.

use std::io::{Read, Write};

use crate::section::MAX_SECTION_COUNT;
use crate::{PersistError, PersistResult};

/// Snapshot file magic.
pub const SNAPSHOT_MAGIC: [u8; 4] = *b"SLSN";
/// Snapshot major format version.
pub const SNAPSHOT_VERSION_MAJOR: u16 = 1;
/// Snapshot minor format version.
///
/// Bumped `0 -> 1` by BRIEF-Item-4a STEP 9: the `CORE/NODE` / `CORE/EDGE`
/// sections now persist the explicit external `NodeId` / `EdgeId` per row
/// instead of synthesizing `row + 1`, and recovery places rows positionally so
/// a future 4b-compacted snapshot (ids != row+1) round-trips. The version gate
/// ([`SnapshotFileHeader::read_from`]) rejects any mismatch, so pre-STEP-9
/// (minor 0) snapshots are cleanly rejected with [`crate::PersistError::UnsupportedVersion`]
/// — a clean break, not a dual decoder (deferred to 4c per the D14 amendment).
pub const SNAPSHOT_VERSION_MINOR: u16 = 1;
/// Fixed snapshot file-header length.
pub const SNAPSHOT_FILE_HEADER_LEN: usize = 32;
/// Whole-body compression flag, reserved in v1.0.
pub const FLAG_BODY_COMPRESSED: u16 = 0b0000_0000_0000_0001;
/// Per-section compression flag.
pub const FLAG_SECTION_COMPRESSED: u16 = 0b0000_0000_0000_0010;

const RESERVED_START_OFFSET: u64 = 12;
const SUPPORTED_FLAGS: u16 = FLAG_SECTION_COMPRESSED;

/// Fixed-size snapshot envelope header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotFileHeader {
    /// Major format version.
    pub version_major: u16,
    /// Minor format version.
    pub version_minor: u16,
    /// Snapshot flags.
    pub flags: u16,
    /// Number of section-table rows.
    pub section_count: u16,
    /// Low 128 bits of blake3 over section table plus payload bytes.
    pub body_hash: [u8; 16],
}

impl SnapshotFileHeader {
    /// Construct a validated v1.0 snapshot file header.
    ///
    /// # Errors
    ///
    /// Returns [`PersistError::TooManySections`] if `section_count` does not
    /// fit the u16 header field or [`PersistError::UnsupportedFlag`] if an
    /// unsupported flag bit is set.
    pub fn new(flags: u16, section_count: usize, body_hash: [u8; 16]) -> PersistResult<Self> {
        validate_flags(flags)?;
        if section_count > MAX_SECTION_COUNT {
            return Err(PersistError::TooManySections {
                count: section_count,
                max: MAX_SECTION_COUNT,
            });
        }
        Ok(Self {
            version_major: SNAPSHOT_VERSION_MAJOR,
            version_minor: SNAPSHOT_VERSION_MINOR,
            flags,
            section_count: section_count as u16,
            body_hash,
        })
    }

    /// Write this fixed 32-byte header.
    pub(crate) fn write_to(&self, writer: &mut impl Write) -> PersistResult<()> {
        writer.write_all(&SNAPSHOT_MAGIC)?;
        writer.write_all(&self.version_major.to_le_bytes())?;
        writer.write_all(&self.version_minor.to_le_bytes())?;
        writer.write_all(&self.flags.to_le_bytes())?;
        writer.write_all(&self.section_count.to_le_bytes())?;
        writer.write_all(&[0_u8; 4])?;
        writer.write_all(&self.body_hash)?;
        Ok(())
    }

    /// Read and validate a fixed 32-byte snapshot header.
    pub(crate) fn read_from(reader: &mut impl Read) -> PersistResult<Self> {
        let mut bytes = [0_u8; SNAPSHOT_FILE_HEADER_LEN];
        reader
            .read_exact(&mut bytes)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::UnexpectedEof => PersistError::TruncatedSnapshotHeader,
                _ => PersistError::Io(error),
            })?;
        let observed = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if observed != SNAPSHOT_MAGIC {
            return Err(PersistError::MagicMismatch { observed });
        }
        let version_major = u16::from_le_bytes([bytes[4], bytes[5]]);
        let version_minor = u16::from_le_bytes([bytes[6], bytes[7]]);
        if version_major != SNAPSHOT_VERSION_MAJOR || version_minor != SNAPSHOT_VERSION_MINOR {
            return Err(PersistError::UnsupportedVersion {
                major: version_major,
                minor: version_minor,
            });
        }
        let flags = u16::from_le_bytes([bytes[8], bytes[9]]);
        validate_flags(flags)?;
        for (index, byte) in bytes[12..16].iter().enumerate() {
            if *byte != 0 {
                return Err(PersistError::ReservedBytesNonZero {
                    offset: RESERVED_START_OFFSET + index as u64,
                });
            }
        }
        let mut body_hash = [0_u8; 16];
        body_hash.copy_from_slice(&bytes[16..32]);
        Ok(Self {
            version_major,
            version_minor,
            flags,
            section_count: u16::from_le_bytes([bytes[10], bytes[11]]),
            body_hash,
        })
    }

    /// Return true if the reserved whole-body compression flag is set.
    #[must_use]
    pub const fn is_body_compressed(&self) -> bool {
        self.flags & FLAG_BODY_COMPRESSED != 0
    }

    /// Return true if section payloads are individually compressed.
    #[must_use]
    pub const fn is_section_compressed(&self) -> bool {
        self.flags & FLAG_SECTION_COMPRESSED != 0
    }
}

fn validate_flags(flags: u16) -> PersistResult<()> {
    if flags & FLAG_BODY_COMPRESSED != 0 {
        return Err(PersistError::UnsupportedFlag {
            flag: FLAG_BODY_COMPRESSED,
        });
    }
    let unsupported = flags & !SUPPORTED_FLAGS;
    if unsupported != 0 {
        return Err(PersistError::UnsupportedFlag { flag: unsupported });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_header_round_trips() {
        let header = SnapshotFileHeader::new(FLAG_SECTION_COMPRESSED, 2, [7_u8; 16]).unwrap();
        let mut bytes = Vec::new();
        header.write_to(&mut bytes).unwrap();
        assert_eq!(bytes.len(), SNAPSHOT_FILE_HEADER_LEN);
        assert_eq!(
            SnapshotFileHeader::read_from(&mut bytes.as_slice()).unwrap(),
            header
        );
        assert!(header.is_section_compressed());
        assert!(!header.is_body_compressed());
    }

    #[test]
    fn magic_mismatch_is_reported() {
        let mut bytes = Vec::new();
        SnapshotFileHeader::new(0, 0, [0; 16])
            .unwrap()
            .write_to(&mut bytes)
            .unwrap();
        bytes[0..4].copy_from_slice(b"NOPE");
        assert!(matches!(
            SnapshotFileHeader::read_from(&mut bytes.as_slice()),
            Err(PersistError::MagicMismatch { observed }) if observed == *b"NOPE"
        ));
    }

    #[test]
    fn unsupported_version_is_reported() {
        let mut bytes = Vec::new();
        SnapshotFileHeader::new(0, 0, [0; 16])
            .unwrap()
            .write_to(&mut bytes)
            .unwrap();
        bytes[4..6].copy_from_slice(&2_u16.to_le_bytes());
        // `new()` writes the current minor (1 after the STEP 9 bump); patching
        // only the major byte leaves minor at its written value.
        assert!(matches!(
            SnapshotFileHeader::read_from(&mut bytes.as_slice()),
            Err(PersistError::UnsupportedVersion { major: 2, minor: 1 })
        ));
    }

    #[test]
    fn pre_step9_minor_zero_is_rejected() {
        // BRIEF-Item-4a STEP 9 clean break: a snapshot written at the previous
        // minor version (0) must fail the gate, since the CORE/NODE & CORE/EDGE
        // section semantics changed. Patch a freshly written (minor 1) header's
        // minor bytes [6..8] back to 0 and confirm a clean UnsupportedVersion
        // rather than a silent mis-decode of the body.
        let mut bytes = Vec::new();
        SnapshotFileHeader::new(0, 0, [0; 16])
            .unwrap()
            .write_to(&mut bytes)
            .unwrap();
        bytes[6..8].copy_from_slice(&0_u16.to_le_bytes());
        assert!(matches!(
            SnapshotFileHeader::read_from(&mut bytes.as_slice()),
            Err(PersistError::UnsupportedVersion { major: 1, minor: 0 })
        ));
    }

    #[test]
    fn reserved_bytes_nonzero_is_reported() {
        let mut bytes = Vec::new();
        SnapshotFileHeader::new(0, 0, [0; 16])
            .unwrap()
            .write_to(&mut bytes)
            .unwrap();
        bytes[13] = 1;
        assert!(matches!(
            SnapshotFileHeader::read_from(&mut bytes.as_slice()),
            Err(PersistError::ReservedBytesNonZero { offset: 13 })
        ));
    }

    #[test]
    fn truncated_header_is_reported() {
        let bytes = [0_u8; SNAPSHOT_FILE_HEADER_LEN - 1];
        assert!(matches!(
            SnapshotFileHeader::read_from(&mut bytes.as_slice()),
            Err(PersistError::TruncatedSnapshotHeader)
        ));
    }

    #[test]
    fn body_compressed_flag_is_rejected() {
        assert!(matches!(
            SnapshotFileHeader::new(FLAG_BODY_COMPRESSED, 0, [0; 16]),
            Err(PersistError::UnsupportedFlag {
                flag: FLAG_BODY_COMPRESSED
            })
        ));
    }

    #[test]
    fn too_many_sections_is_rejected() {
        assert!(matches!(
            SnapshotFileHeader::new(0, MAX_SECTION_COUNT + 1, [0; 16]),
            Err(PersistError::TooManySections {
                count,
                max: MAX_SECTION_COUNT
            }) if count == MAX_SECTION_COUNT + 1
        ));
    }
}
