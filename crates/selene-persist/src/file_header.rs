//! Fixed WAL file header.

use std::io::{Read, Write};

use crate::{PersistError, PersistResult};

/// WAL file magic.
pub const WAL_MAGIC: [u8; 4] = *b"SLDB";
/// WAL major format version.
///
/// Bumped `2 -> 3` when the frame layout came under integrity protection: the
/// file header grew a reserved word and a checksum, and every entry gained a
/// prefix checksum over its framing fields plus an extent checksum over its
/// provenance tail and principal. Before that, a single bit flip in
/// `payload_len` made a mid-file frame look truncated and the writer silently
/// discarded every committed frame after it.
///
/// There is no dual decoder and no migrator. A store written by 1.0.0-1.4.0
/// (WAL 2.0 or 2.2) is rejected by the version gate with a typed
/// `UnsupportedVersion` before anything on disk is touched; recreate it from
/// source. See `docs/persistence-and-recovery.md` for the shipped-identity
/// table and the compatibility policy.
pub const WAL_VERSION_MAJOR: u16 = 3;
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
///
/// Typed checkpoint watermarks did not bump this version: they use the existing
/// entry-header flag field plus the already-valid empty `Vec<Change>` payload.
/// Pre-watermark v2.2 readers preserve graph data but can conservatively fold a
/// physical watermark into graph generation, so exact generation continuity
/// across a downgrade is not supported after a coordinated checkpoint.
pub const WAL_VERSION_MINOR: u16 = 0;
/// Fixed WAL file header length.
pub const WAL_FILE_HEADER_LEN: usize = 24;

/// Bytes of the file header covered by its checksum: everything before it.
const WAL_FILE_HEADER_CHECKSUM_OFFSET: usize = 20;

/// Bytes read before the version gate runs.
///
/// The gate must answer "wrong version" for a v2 store, including a
/// header-only one whose whole file is 16 bytes — shorter than a v3 header. So
/// the magic and version are read and judged on their own, before any read
/// that a v2 file is too short to satisfy.
const WAL_VERSION_PREFIX_LEN: usize = 8;

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
    /// Construct a WAL file header at the current format version.
    #[must_use]
    pub const fn new(snapshot_seq: u64) -> Self {
        Self {
            version_major: WAL_VERSION_MAJOR,
            version_minor: WAL_VERSION_MINOR,
            snapshot_seq,
        }
    }

    /// Write this fixed 24-byte header.
    pub(crate) fn write_to(&self, writer: &mut impl Write) -> PersistResult<()> {
        let mut bytes = [0u8; WAL_FILE_HEADER_LEN];
        bytes[0..4].copy_from_slice(&WAL_MAGIC);
        bytes[4..6].copy_from_slice(&self.version_major.to_le_bytes());
        bytes[6..8].copy_from_slice(&self.version_minor.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.snapshot_seq.to_le_bytes());
        // bytes[16..20] stay zero: reserved.
        let checksum = crate::payload::checksum_lo(&bytes[..WAL_FILE_HEADER_CHECKSUM_OFFSET]);
        bytes[WAL_FILE_HEADER_CHECKSUM_OFFSET..].copy_from_slice(&checksum.to_le_bytes());
        writer.write_all(&bytes)?;
        Ok(())
    }

    /// Read and validate the fixed 24-byte header.
    ///
    /// Order is load-bearing. Magic and version are judged from the first eight
    /// bytes alone, so a store written by a released 2.x build — whose entire
    /// header may be shorter than this one — is reported as an unsupported
    /// version rather than as a truncated or corrupt file. Only then is the
    /// rest of the header read and its checksum verified.
    pub(crate) fn read_from(reader: &mut impl Read) -> PersistResult<Self> {
        let mut bytes = [0u8; WAL_FILE_HEADER_LEN];
        read_exact_or_truncated(reader, &mut bytes[..WAL_VERSION_PREFIX_LEN])?;
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
        read_exact_or_truncated(reader, &mut bytes[WAL_VERSION_PREFIX_LEN..])?;

        let expected = crate::payload::checksum_lo(&bytes[..WAL_FILE_HEADER_CHECKSUM_OFFSET]);
        let stored = u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        if stored != expected {
            return Err(PersistError::WalFileHeaderChecksumMismatch);
        }
        // Checked after the checksum, so a nonzero reserved word is reported as
        // a forward-compatibility signal rather than as corruption.
        if bytes[16..20] != [0, 0, 0, 0] {
            return Err(PersistError::ReservedBytesNonZero { offset: 16 });
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

fn read_exact_or_truncated(reader: &mut impl Read, buf: &mut [u8]) -> PersistResult<()> {
    reader.read_exact(buf).map_err(|err| match err.kind() {
        std::io::ErrorKind::UnexpectedEof => PersistError::TruncatedFileHeader,
        _ => PersistError::Io(err),
    })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

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

    /// Every rejected version is expressed RELATIVE to the current one, so a
    /// future bump cannot silently turn a rejection case into an accepted one.
    /// The previous suite hard-coded `major: 3` as "future"; bumping to 3.0
    /// made that header valid and the test then panicked with no explanation.
    #[rstest]
    #[case(WAL_VERSION_MAJOR - 1, WAL_VERSION_MINOR)]
    #[case(WAL_VERSION_MAJOR + 1, WAL_VERSION_MINOR)]
    #[case(0, WAL_VERSION_MINOR)]
    #[case(WAL_VERSION_MAJOR, WAL_VERSION_MINOR + 1)]
    fn non_current_versions_are_rejected(#[case] major: u16, #[case] minor: u16) {
        assert!(
            (major, minor) != (WAL_VERSION_MAJOR, WAL_VERSION_MINOR),
            "a rejection case must not name the current version"
        );
        let mut bytes = Vec::new();
        WalFileHeader::new(0).write_to(&mut bytes).unwrap();
        bytes[4..6].copy_from_slice(&major.to_le_bytes());
        bytes[6..8].copy_from_slice(&minor.to_le_bytes());
        let observed = WalFileHeader::read_from(&mut bytes.as_slice());
        assert!(
            matches!(
                observed,
                Err(PersistError::UnsupportedVersion { major: m, minor: n }) if m == major && n == minor
            ),
            "expected UnsupportedVersion({major}, {minor}), got {observed:?}"
        );
    }

    /// The one error a consumer upgrading from a released 1.x build is
    /// guaranteed to hit. It must name the version, not report the file as
    /// truncated (a v2 header-only WAL is 16 bytes, shorter than a v3 header)
    /// and not report a checksum failure (v2 wrote no checksum).
    #[rstest]
    #[case::header_only(vec![])]
    #[case::with_frame_bytes(vec![0xAB; 8])]
    fn released_v2_stores_are_rejected_as_unsupported(#[case] trailing: Vec<u8>) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&WAL_MAGIC);
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&7u64.to_le_bytes());
        bytes.extend_from_slice(&trailing);
        let observed = WalFileHeader::read_from(&mut bytes.as_slice());
        assert!(
            matches!(
                observed,
                Err(PersistError::UnsupportedVersion { major: 2, minor: 2 })
            ),
            "a v1.2.0-v1.4.0 store must be diagnosed by version, got {observed:?}"
        );
    }

    /// The framing the version gate cannot see is covered by the checksum.
    #[rstest]
    #[case::snapshot_seq(8)]
    #[case::snapshot_seq_high(15)]
    #[case::reserved(16)]
    fn file_header_corruption_is_rejected(#[case] index: usize) {
        let mut bytes = Vec::new();
        WalFileHeader::new(0x0102_0304_0506_0708)
            .write_to(&mut bytes)
            .unwrap();
        bytes[index] ^= 0b0000_0001;
        let observed = WalFileHeader::read_from(&mut bytes.as_slice());
        assert!(
            matches!(observed, Err(PersistError::WalFileHeaderChecksumMismatch)),
            "byte {index} must be under integrity protection, got {observed:?}"
        );
    }

    #[test]
    fn wal_format_identity_is_pinned() {
        // Guards the atomicity hazard: a frame-layout change that lands without
        // a version bump produces an on-disk identity indistinguishable from a
        // released build's.
        assert_eq!((WAL_VERSION_MAJOR, WAL_VERSION_MINOR), (3, 0));
        assert_eq!(WAL_FILE_HEADER_LEN, 24);
    }

    /// Truncation is reported at both read stages: before the version prefix is
    /// complete, and after it when the rest of the header is short.
    #[rstest]
    #[case::before_the_version_prefix(WAL_VERSION_PREFIX_LEN - 1)]
    #[case::after_the_version_prefix(WAL_FILE_HEADER_LEN - 1)]
    fn truncated_header_is_reported(#[case] len: usize) {
        let mut bytes = Vec::new();
        WalFileHeader::new(0).write_to(&mut bytes).unwrap();
        bytes.truncate(len);
        assert!(matches!(
            WalFileHeader::read_from(&mut bytes.as_slice()),
            Err(PersistError::TruncatedFileHeader)
        ));
    }
}
