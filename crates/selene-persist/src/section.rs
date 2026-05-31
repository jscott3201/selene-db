//! Snapshot section table rows.

use std::io::{Read, Write};

use crate::{PersistError, PersistResult};

/// Snapshot section-table row length in bytes.
pub const SECTION_TABLE_ROW_LEN: usize = 24;
/// Hard per-section payload cap.
pub const MAX_SECTION_PAYLOAD_BYTES: usize = 1_usize << 30;
/// Maximum number of sections representable by the snapshot header.
pub const MAX_SECTION_COUNT: usize = u16::MAX as usize;

/// One snapshot section-table entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SectionEntry {
    /// Four-byte provider tag.
    pub provider: [u8; 4],
    /// Four-byte provider-owned sub-tag.
    pub sub: [u8; 4],
    /// File-absolute byte offset of this section's payload.
    pub payload_offset: u64,
    /// On-disk payload length in bytes.
    pub payload_len: u64,
}

impl SectionEntry {
    /// Return the composite tag pair.
    #[must_use]
    pub const fn tag_pair(&self) -> ([u8; 4], [u8; 4]) {
        (self.provider, self.sub)
    }
}

pub(crate) fn write_section_table(
    writer: &mut impl Write,
    entries: &[SectionEntry],
) -> PersistResult<()> {
    for entry in entries {
        writer.write_all(&entry.provider)?;
        writer.write_all(&entry.sub)?;
        writer.write_all(&entry.payload_offset.to_le_bytes())?;
        writer.write_all(&entry.payload_len.to_le_bytes())?;
    }
    Ok(())
}

pub(crate) fn section_table_bytes(entries: &[SectionEntry]) -> PersistResult<Vec<u8>> {
    let mut bytes = Vec::with_capacity(entries.len() * SECTION_TABLE_ROW_LEN);
    write_section_table(&mut bytes, entries)?;
    Ok(bytes)
}

pub(crate) fn body_hash<B>(section_table: &[u8], payloads: impl IntoIterator<Item = B>) -> [u8; 16]
where
    B: AsRef<[u8]>,
{
    let mut hasher = blake3::Hasher::new();
    hasher.update(section_table);
    for payload in payloads {
        hasher.update(payload.as_ref());
    }
    let digest = hasher.finalize();
    let mut hash = [0_u8; 16];
    hash.copy_from_slice(&digest.as_bytes()[..16]);
    hash
}

pub(crate) fn read_section_table(
    reader: &mut impl Read,
    section_count: usize,
) -> PersistResult<Vec<SectionEntry>> {
    let mut entries = Vec::with_capacity(section_count);
    for index in 0..section_count {
        let offset =
            crate::SNAPSHOT_FILE_HEADER_LEN as u64 + (index as u64 * SECTION_TABLE_ROW_LEN as u64);
        let mut row = [0_u8; SECTION_TABLE_ROW_LEN];
        reader
            .read_exact(&mut row)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::UnexpectedEof => PersistError::TruncatedSectionTable { offset },
                _ => PersistError::Io(error),
            })?;
        let payload_len = u64::from_le_bytes([
            row[16], row[17], row[18], row[19], row[20], row[21], row[22], row[23],
        ]);
        validate_section_payload_len_u64(payload_len)?;
        entries.push(SectionEntry {
            provider: [row[0], row[1], row[2], row[3]],
            sub: [row[4], row[5], row[6], row[7]],
            payload_offset: u64::from_le_bytes([
                row[8], row[9], row[10], row[11], row[12], row[13], row[14], row[15],
            ]),
            payload_len,
        });
    }
    Ok(entries)
}

/// Validate a snapshot section payload length.
///
/// # Errors
///
/// Returns [`PersistError::SectionTooLarge`] if `len` exceeds 1 GiB.
pub fn validate_section_payload_len(len: usize) -> PersistResult<()> {
    if len > MAX_SECTION_PAYLOAD_BYTES {
        return Err(PersistError::SectionTooLarge {
            len,
            max: MAX_SECTION_PAYLOAD_BYTES,
        });
    }
    Ok(())
}

pub(crate) fn validate_section_payload_len_u64(len: u64) -> PersistResult<()> {
    if len > MAX_SECTION_PAYLOAD_BYTES as u64 {
        let len = usize::try_from(len).unwrap_or(usize::MAX);
        return Err(PersistError::SectionTooLarge {
            len,
            max: MAX_SECTION_PAYLOAD_BYTES,
        });
    }
    validate_section_payload_len(len as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_table_round_trips() {
        let entries = vec![
            SectionEntry {
                provider: *b"CORE",
                sub: *b"META",
                payload_offset: 80,
                payload_len: 7,
            },
            SectionEntry {
                provider: *b"DEMO",
                sub: *b"SUBT",
                payload_offset: 87,
                payload_len: 11,
            },
        ];
        let bytes = section_table_bytes(&entries).unwrap();
        assert_eq!(bytes.len(), SECTION_TABLE_ROW_LEN * entries.len());
        assert_eq!(
            read_section_table(&mut bytes.as_slice(), entries.len()).unwrap(),
            entries
        );
    }

    #[test]
    fn truncated_table_reports_row_offset() {
        let bytes = vec![0_u8; SECTION_TABLE_ROW_LEN + 2];
        let err = read_section_table(&mut bytes.as_slice(), 2).unwrap_err();
        assert!(matches!(
            err,
            PersistError::TruncatedSectionTable { offset }
                if offset == (crate::SNAPSHOT_FILE_HEADER_LEN + SECTION_TABLE_ROW_LEN) as u64
        ));
    }

    #[test]
    fn validate_section_payload_len_allows_max_and_rejects_overflow() {
        validate_section_payload_len(MAX_SECTION_PAYLOAD_BYTES).unwrap();
        let err = validate_section_payload_len(MAX_SECTION_PAYLOAD_BYTES + 1).unwrap_err();
        assert!(matches!(
            err,
            PersistError::SectionTooLarge {
                len,
                max: MAX_SECTION_PAYLOAD_BYTES
            } if len == MAX_SECTION_PAYLOAD_BYTES + 1
        ));
    }

    #[test]
    fn read_revalidates_payload_len_cap() {
        let entry = SectionEntry {
            provider: *b"CORE",
            sub: *b"META",
            payload_offset: 56,
            payload_len: MAX_SECTION_PAYLOAD_BYTES as u64 + 1,
        };
        let bytes = section_table_bytes(&[entry]).unwrap();
        assert!(matches!(
            read_section_table(&mut bytes.as_slice(), 1),
            Err(PersistError::SectionTooLarge {
                max: MAX_SECTION_PAYLOAD_BYTES,
                ..
            })
        ));
    }
}
