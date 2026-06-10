//! Fixed WAL file header.

use std::io::{Read, Write};

use crate::{PersistError, PersistResult};

/// WAL file magic.
pub const WAL_MAGIC: [u8; 4] = *b"SLDB";
/// WAL major format version.
pub const WAL_VERSION_MAJOR: u16 = 2;
/// WAL minor format version.
///
/// Bumped `0 -> 1` when the serialized `Value` enum layout changed. WAL change
/// payloads (`payload.rs`) postcard-encode `Value`, and the version gate
/// (`WalFileHeader::read_from`) rejects any mismatch rather than silently
/// mis-decoding shifted variant discriminants — a clean greenfield break, not a
/// dual decoder.
///
/// Bumped `1 -> 2` by the typed-descriptor stream: `ValueType` (postcard-encoded
/// inside `SchemaChange::NodeTypeAddedV2` / `EdgeTypeAddedV2` definitions in WAL
/// change payloads) gained mid-struct `decimal_type` / `character_string_type` /
/// `byte_string_type` fields, and `RecordFieldStructureType` gained descriptor
/// variants ahead of `List` / `Record`. `#[serde(default)]` is a no-op for
/// postcard's positional decode, so pre-descriptor WALs must be rejected by the
/// version gate rather than mis-decoded as shifted fields/discriminants.
pub const WAL_VERSION_MINOR: u16 = 2;
/// Fixed WAL file header length.
pub const WAL_FILE_HEADER_LEN: usize = 16;

/// Fixed-size WAL file envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WalFileHeader {
    /// Major format version.
    pub version_major: u16,
    /// Minor format version.
    pub version_minor: u16,
    /// Highest WAL sequence covered by the snapshot this WAL extends.
    pub snapshot_seq: u64,
}

impl WalFileHeader {
    /// Construct a v2.0 WAL file header.
    #[must_use]
    pub const fn new(snapshot_seq: u64) -> Self {
        Self {
            version_major: WAL_VERSION_MAJOR,
            version_minor: WAL_VERSION_MINOR,
            snapshot_seq,
        }
    }

    /// Write this fixed 16-byte header.
    pub(crate) fn write_to(&self, writer: &mut impl Write) -> PersistResult<()> {
        writer.write_all(&WAL_MAGIC)?;
        writer.write_all(&self.version_major.to_le_bytes())?;
        writer.write_all(&self.version_minor.to_le_bytes())?;
        writer.write_all(&self.snapshot_seq.to_le_bytes())?;
        Ok(())
    }

    /// Read and validate a fixed 16-byte header.
    pub(crate) fn read_from(reader: &mut impl Read) -> PersistResult<Self> {
        let mut bytes = [0u8; WAL_FILE_HEADER_LEN];
        reader
            .read_exact(&mut bytes)
            .map_err(|err| match err.kind() {
                std::io::ErrorKind::UnexpectedEof => PersistError::TruncatedFileHeader,
                _ => PersistError::Io(err),
            })?;
        let observed = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if observed != WAL_MAGIC {
            return Err(PersistError::MagicMismatch { observed });
        }
        let version_major = u16::from_le_bytes([bytes[4], bytes[5]]);
        let version_minor = u16::from_le_bytes([bytes[6], bytes[7]]);
        if version_major != WAL_VERSION_MAJOR || version_minor != WAL_VERSION_MINOR {
            return Err(PersistError::UnsupportedVersion {
                major: version_major,
                minor: version_minor,
            });
        }
        let snapshot_seq = u64::from_le_bytes([
            bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
        ]);
        Ok(Self {
            version_major,
            version_minor,
            snapshot_seq,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_header_round_trips() {
        let header = WalFileHeader::new(42);
        let mut bytes = Vec::new();
        header.write_to(&mut bytes).unwrap();
        assert_eq!(bytes.len(), WAL_FILE_HEADER_LEN);
        assert_eq!(
            WalFileHeader::read_from(&mut bytes.as_slice()).unwrap(),
            header
        );
    }

    #[test]
    fn magic_mismatch_is_reported() {
        let mut bytes = Vec::new();
        WalFileHeader::new(0).write_to(&mut bytes).unwrap();
        bytes[0..4].copy_from_slice(b"NOPE");
        assert!(matches!(
            WalFileHeader::read_from(&mut bytes.as_slice()),
            Err(PersistError::MagicMismatch { observed }) if observed == *b"NOPE"
        ));
    }

    #[test]
    fn unsupported_version_is_reported() {
        let mut bytes = Vec::new();
        WalFileHeader::new(0).write_to(&mut bytes).unwrap();
        bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
        // `new()` writes the current minor (2 after the descriptor bump);
        // patching only the major byte leaves minor at its written value.
        assert!(matches!(
            WalFileHeader::read_from(&mut bytes.as_slice()),
            Err(PersistError::UnsupportedVersion { major: 1, minor: 2 })
        ));
    }

    #[test]
    fn v3_future_version_is_rejected() {
        let mut bytes = Vec::new();
        WalFileHeader::new(0).write_to(&mut bytes).unwrap();
        bytes[4..6].copy_from_slice(&3u16.to_le_bytes());
        assert!(matches!(
            WalFileHeader::read_from(&mut bytes.as_slice()),
            Err(PersistError::UnsupportedVersion { major: 3, minor: 2 })
        ));
    }

    #[test]
    fn zero_version_is_unsupported() {
        let mut bytes = Vec::new();
        WalFileHeader::new(0).write_to(&mut bytes).unwrap();
        bytes[4..6].copy_from_slice(&0u16.to_le_bytes());
        assert!(matches!(
            WalFileHeader::read_from(&mut bytes.as_slice()),
            Err(PersistError::UnsupportedVersion { major: 0, minor: 2 })
        ));
    }

    #[test]
    fn pre_descriptor_minor_one_is_rejected() {
        // Typed-descriptor clean break: a WAL written at the previous minor
        // version (1) postcard-encodes `ValueType` WITHOUT the descriptor fields
        // (and `RecordFieldStructureType` without the descriptor variants), so it
        // must fail the gate with a clean UnsupportedVersion rather than
        // mis-decoding shifted fields/discriminants. Patch a freshly written
        // (minor 2) header's minor bytes [6..8] back to 1.
        let mut bytes = Vec::new();
        WalFileHeader::new(0).write_to(&mut bytes).unwrap();
        bytes[6..8].copy_from_slice(&1u16.to_le_bytes());
        assert!(matches!(
            WalFileHeader::read_from(&mut bytes.as_slice()),
            Err(PersistError::UnsupportedVersion { major: 2, minor: 1 })
        ));
    }

    #[test]
    fn truncated_header_is_reported() {
        let bytes = [0u8; WAL_FILE_HEADER_LEN - 1];
        assert!(matches!(
            WalFileHeader::read_from(&mut bytes.as_slice()),
            Err(PersistError::TruncatedFileHeader)
        ));
    }
}
