//! Audit-record scanning: torn tails are repaired, corruption before the tail
//! refuses.
//!
//! The audit twin of [`crate::wal_tail`], and it answers the same question the
//! same way: a record that fails validation is a torn tail only if **nothing
//! follows it**. Append is the only growth operation, so trailing bytes prove
//! the failing record was complete before them, which makes the failure damage
//! rather than an interrupted write.
//!
//! Format v1 could not ask that question. Its only checksum covered the
//! payload, so `payload_len` — the sole field saying where a record ends — was
//! unprotected, and "does anything follow this record?" was unanswerable
//! exactly when the answer mattered. v2 checksums the record header, so a
//! record's declared extent is trustworthy before it is used as a length or an
//! offset. That is the property WAL v3's prefix checksum provides, and it is
//! the prerequisite this classifier is built on.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use super::{
    AUDIT_FILE_HEADER_LEN, AUDIT_RECORD_HEADER_LEN, AuditRecord, MAX_AUDIT_PAYLOAD_BYTES,
    RecordHeader, decode_record_header, verify_prefix_checksum,
};
use crate::{PersistError, PersistResult};

/// Chunk size for the zero probe over an untrusted region.
const ZERO_PROBE_CHUNK: usize = 64 * 1024;

/// A byte source the scanner can walk: an open file, or an in-memory slice.
///
/// The two exist so the fuzz target can drive the identical classifier over
/// arbitrary bytes without a file handle. Keeping one classifier and two
/// sources is what stops the twins from drifting — a divergence
/// `tests/decode_from_bytes_equivalence.rs` pins but could not prevent while
/// each twin carried its own copy of the rules.
trait RecordSource {
    /// Total length of the source in bytes.
    fn len(&self) -> u64;

    /// Read the fixed record header at `offset`, or `None` if it does not fit.
    fn read_header(&mut self, offset: u64) -> PersistResult<Option<[u8; AUDIT_RECORD_HEADER_LEN]>>;

    /// Read exactly `len` payload bytes starting at `offset`.
    fn read_payload(&mut self, offset: u64, len: usize) -> PersistResult<Option<Vec<u8>>>;

    /// True when every byte from `offset` to the end is zero.
    fn all_zeros_from(&mut self, offset: u64) -> PersistResult<bool>;
}

struct FileSource<'a> {
    file: &'a mut File,
    len: u64,
}

impl RecordSource for FileSource<'_> {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_header(&mut self, offset: u64) -> PersistResult<Option<[u8; AUDIT_RECORD_HEADER_LEN]>> {
        self.file.seek(SeekFrom::Start(offset))?;
        let mut bytes = [0_u8; AUDIT_RECORD_HEADER_LEN];
        match self.file.read_exact(&mut bytes) {
            Ok(()) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(error) => Err(PersistError::Io(error)),
        }
    }

    fn read_payload(&mut self, offset: u64, len: usize) -> PersistResult<Option<Vec<u8>>> {
        self.file.seek(SeekFrom::Start(offset))?;
        let mut payload = vec![0_u8; len];
        match self.file.read_exact(&mut payload) {
            Ok(()) => Ok(Some(payload)),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(error) => Err(PersistError::Io(error)),
        }
    }

    fn all_zeros_from(&mut self, offset: u64) -> PersistResult<bool> {
        self.file.seek(SeekFrom::Start(offset))?;
        let mut remaining = self.len.saturating_sub(offset);
        let mut buffer = vec![0_u8; ZERO_PROBE_CHUNK];
        while remaining > 0 {
            let want = remaining.min(ZERO_PROBE_CHUNK as u64) as usize;
            let slice = &mut buffer[..want];
            if self.file.read_exact(slice).is_err() {
                return Ok(false);
            }
            if slice.iter().any(|byte| *byte != 0) {
                return Ok(false);
            }
            remaining -= want as u64;
        }
        Ok(true)
    }
}

struct SliceSource<'a> {
    bytes: &'a [u8],
}

impl RecordSource for SliceSource<'_> {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_header(&mut self, offset: u64) -> PersistResult<Option<[u8; AUDIT_RECORD_HEADER_LEN]>> {
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let end = match start.checked_add(AUDIT_RECORD_HEADER_LEN) {
            Some(end) => end,
            None => return Ok(None),
        };
        Ok(self
            .bytes
            .get(start..end)
            .map(|slice| slice.try_into().expect("checked slice length")))
    }

    fn read_payload(&mut self, offset: u64, len: usize) -> PersistResult<Option<Vec<u8>>> {
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let end = match start.checked_add(len) {
            Some(end) => end,
            None => return Ok(None),
        };
        Ok(self.bytes.get(start..end).map(<[u8]>::to_vec))
    }

    fn all_zeros_from(&mut self, offset: u64) -> PersistResult<bool> {
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        Ok(self
            .bytes
            .get(start..)
            .is_some_and(|rest| rest.iter().all(|byte| *byte == 0)))
    }
}

/// Where a record ends, once its header is known to be authentic.
struct Extent {
    header: RecordHeader,
    payload_start: u64,
    payload_end: u64,
}

/// Wrap `source` as a refusal naming the record that failed and how much
/// followed it.
fn refuse<T>(offset: u64, file_len: u64, source: PersistError) -> PersistResult<T> {
    Err(PersistError::AuditMidLogCorruption {
        offset,
        trailing_bytes: file_len.saturating_sub(offset),
        source: Box::new(source),
    })
}

/// Authenticate the header at `offset` and compute the record's extent.
///
/// `Ok(None)` means the region from `offset` is a genuine tail. Any failure
/// with records after it refuses instead.
fn read_extent(source: &mut dyn RecordSource, offset: u64) -> PersistResult<Option<Extent>> {
    let file_len = source.len();
    // A header that does not fit cannot have anything after it, so it is a tear
    // by construction and needs no probe.
    if file_len.saturating_sub(offset) < AUDIT_RECORD_HEADER_LEN as u64 {
        return Ok(None);
    }
    let Some(bytes) = source.read_header(offset)? else {
        return Ok(None);
    };

    if !verify_prefix_checksum(&bytes) {
        // The extent is unknown, so position cannot classify this. The only
        // remaining evidence is whether the rest of the file is the zero fill a
        // short extending write leaves.
        return if source.all_zeros_from(offset)? {
            Ok(None)
        } else {
            refuse(
                offset,
                file_len,
                PersistError::AuditHeaderChecksumMismatch { offset },
            )
        };
    }

    // From here the extent IS trustworthy: payload_len came from bytes the
    // header checksum covered, so every later failure is classifiable by where
    // the record ends rather than by a probe.
    let header = decode_record_header(&bytes);
    if header.payload_len > MAX_AUDIT_PAYLOAD_BYTES {
        // An authenticated header cannot be over-cap: `encode_record` rejects
        // such a payload before writing. This is a forged or foreign record,
        // not an interrupted append.
        return refuse(
            offset,
            file_len,
            PersistError::PayloadTooLarge {
                len: header.payload_len,
                max: MAX_AUDIT_PAYLOAD_BYTES,
            },
        );
    }
    let payload_start = offset.saturating_add(AUDIT_RECORD_HEADER_LEN as u64);
    let payload_end = payload_start.saturating_add(header.payload_len as u64);
    if payload_end > file_len {
        // The record runs past end of file, so nothing follows it: a tear.
        return Ok(None);
    }
    Ok(Some(Extent {
        header,
        payload_start,
        payload_end,
    }))
}

/// Read the record at `offset` from any source.
///
/// `Ok(Some((record, next_offset)))` for a good record; `Ok(None)` at a clean
/// end or a genuine torn tail; `Err` when the failure has records after it.
fn read_one(
    source: &mut dyn RecordSource,
    offset: u64,
) -> PersistResult<Option<(AuditRecord, u64)>> {
    let file_len = source.len();
    if offset >= file_len {
        return Ok(None);
    }
    let Some(extent) = read_extent(source, offset)? else {
        return Ok(None);
    };
    let Some(payload) = source.read_payload(extent.payload_start, extent.header.payload_len)?
    else {
        return Ok(None);
    };
    if xxhash_rust::xxh3::xxh3_64(&payload) as u32 != extent.header.payload_checksum_lo {
        // The extent is trustworthy, so position decides: a record with bytes
        // after it was complete before them and cannot be a torn write.
        return if extent.payload_end < file_len {
            refuse(
                offset,
                file_len,
                PersistError::AuditRecordChecksumMismatch { offset },
            )
        } else {
            Ok(None)
        };
    }
    Ok(Some((
        AuditRecord {
            recorded_at_unix_nanos: extent.header.recorded_at_unix_nanos,
            kind: extent.header.kind,
            payload,
        },
        extent.payload_end,
    )))
}

/// Scan from the file header to the durable end.
///
/// Returns the offset where the next append belongs. Returns
/// [`PersistError::AuditMidLogCorruption`] — proposing no truncation — when the
/// failure cannot be a torn tail. The caller must not modify the file in that
/// case; that is the whole point of the classification.
pub(super) fn scan_durable_end(file: &mut File, file_len: u64) -> PersistResult<u64> {
    let mut source = FileSource {
        file,
        len: file_len,
    };
    let mut offset = AUDIT_FILE_HEADER_LEN as u64;
    while let Some((_, next)) = read_one(&mut source, offset)? {
        offset = next;
    }
    Ok(offset)
}

/// Stream every durable record from the file, oldest first.
///
/// Peak memory is the decoded records plus the single payload being read, not
/// the whole file. Refuses on interior corruption rather than stopping early,
/// so a reader cannot silently return a prefix of the history.
pub(super) fn read_records(file: &mut File, file_len: u64) -> PersistResult<Vec<AuditRecord>> {
    let mut source = FileSource {
        file,
        len: file_len,
    };
    collect(&mut source)
}

/// Scan every durable record from an in-memory slice — the slice twin of
/// [`read_records`], used by the decoder fuzz target.
///
/// Every bound is checked against the slice length before the slice is taken,
/// so the invariant "arbitrary bytes never panic" holds.
pub(super) fn read_records_bytes(bytes: &[u8]) -> PersistResult<Vec<AuditRecord>> {
    let mut source = SliceSource { bytes };
    collect(&mut source)
}

fn collect(source: &mut dyn RecordSource) -> PersistResult<Vec<AuditRecord>> {
    let mut out = Vec::new();
    let mut offset = AUDIT_FILE_HEADER_LEN as u64;
    while let Some((record, next)) = read_one(source, offset)? {
        out.push(record);
        offset = next;
    }
    Ok(out)
}
