//! Append-only audit log (`audit.log`, `SLAU`): durable engine-event record
//! with retention independent of the WAL + snapshot lineage.
//!
//! Per the 2026-05-26 deletion+reclamation audit (Item 7 / Seam D), engine
//! events that were written into the WAL change stream got moved by
//! `WalWriter::rotate` into `wal.{N}.archive` files, which nothing reads — so an
//! embedder that pruned archives (Item 5) silently lost that history. The audit
//! log fixes this by recording engine-owned events in a dedicated file with its
//! own [`AuditRetentionPolicy`], decoupled from WAL-archive pruning. It is the
//! durable "events" surface in the snapshot=state / WAL=changes / audit=events
//! split, available as a forward framework for user-action audit events.
//!
//! # Layering: clock- and semantics-agnostic
//!
//! This module is below `selene-graph` and never interprets event payloads. A
//! record is an opaque `kind`-tagged byte blob plus a caller-supplied wall-clock
//! stamp ([`AuditRecord::recorded_at_unix_nanos`]). The caller (the graph
//! mutation funnel) stamps the time and serializes the typed event into the
//! payload, so `audit.rs` stays deterministic in tests and free of any clock or
//! lifecycle dependency. `kind` tags are reserved here (e.g.
//! [`AUDIT_KIND_PACK_LIFECYCLE`]); the meaning of the payload lives one layer up.
//!
//! # Format (`format_version = 1`)
//!
//! ```text
//! file header (hand-rolled LE, 8 bytes):
//!   [0..4) magic = b"SLAU"
//!   [4..6) format_version: u16 LE = 1
//!   [6..8) reserved:       u16 LE = 0  (zero-checked on read)
//! then zero or more append-only records, each:
//!   fixed record header (LE, 20 bytes):
//!     recorded_at_unix_nanos: u64 LE
//!     kind:                   u16 LE
//!     reserved:               u16 LE = 0
//!     payload_len:            u32 LE   (<= MAX_AUDIT_PAYLOAD_BYTES)
//!     checksum_lo:            u32 LE   (low 32 of xxh3_64(payload))
//!   payload: payload_len opaque bytes
//! ```
//!
//! # Crash safety
//!
//! Append is the only growth operation; a power loss can only tear the final
//! record. [`AuditLog::open`] scans from the file header and truncates at the
//! first short / over-cap / checksum-failed record, mirroring the WAL's
//! torn-tail recovery (`writer::scan_existing`). [`AuditLog::prune`] rewrites
//! the whole log via the write-tmp → fsync → atomic-rename → dir-fsync idiom
//! ([`crate::manifest::Manifest::write_atomic`]'s pattern), so a crash mid-prune
//! leaves the prior `audit.log` fully intact.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::manifest::sync_dir;
use crate::{PersistError, PersistResult};

/// Audit-log file magic.
pub const AUDIT_MAGIC: [u8; 4] = *b"SLAU";
/// Audit-log format version understood by this build.
pub const AUDIT_FORMAT_VERSION: u16 = 1;
/// Conventional audit-log file name used by embedders.
pub const DEFAULT_AUDIT_FILE_NAME: &str = "audit.log";
/// Maximum opaque payload bytes per audit record (1 MiB).
pub const MAX_AUDIT_PAYLOAD_BYTES: usize = 1 << 20;
/// Reserved `kind` tag (value `1`), historically used for procedure-pack
/// lifecycle events. The pack producer was removed in the extension teardown;
/// the tag stays reserved so the `kind` space stays stable for the forward
/// user-action audit framework. The payload meaning lives in the layer that
/// writes it; `audit.rs` only stores the tag.
pub const AUDIT_KIND_PACK_LIFECYCLE: u16 = 1;

const AUDIT_FILE_HEADER_LEN: usize = 8;
const AUDIT_RECORD_HEADER_LEN: usize = 20;

/// One durable audit record.
///
/// `recorded_at_unix_nanos` is a caller-supplied wall-clock stamp (nanoseconds
/// since the Unix epoch) used for age-based retention and ordering; this crate
/// never reads the system clock itself. `kind` tags the payload's schema (see
/// [`AUDIT_KIND_PACK_LIFECYCLE`]); `payload` is opaque to this layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditRecord {
    /// Wall-clock append time, nanoseconds since the Unix epoch (caller-stamped).
    pub recorded_at_unix_nanos: u64,
    /// Payload schema tag.
    pub kind: u16,
    /// Opaque event bytes.
    pub payload: Vec<u8>,
}

/// Typed retention policy for the audit log.
///
/// Both constraints are **conjunctive**: a record is retained only if it is
/// among the newest [`keep_n_events`](Self::keep_n_events) *and* not older than
/// [`max_age`](Self::max_age). `None` disables a constraint. The default retains
/// everything — pack-lifecycle events are sparse, so unbounded growth is the
/// safe default and the embedder opts into trimming.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AuditRetentionPolicy {
    /// Maximum records to retain (newest kept). `None` = unbounded.
    pub keep_n_events: Option<u64>,
    /// Maximum record age relative to the prune's `now`. `None` = no age limit.
    pub max_age: Option<Duration>,
}

/// What an [`AuditLog::prune`] retained and removed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AuditPruneOutcome {
    /// Number of records retained.
    pub retained: u64,
    /// Number of records removed.
    pub removed: u64,
    /// Bytes reclaimed by the rewrite.
    pub bytes_reclaimed: u64,
}

/// Append-only audit-log writer.
///
/// Holds an exclusive append handle positioned at the durable tail. Construct
/// with [`AuditLog::open`]; append with [`AuditLog::append`]; read the full
/// history with [`AuditLog::read_all`]; trim with [`AuditLog::prune`].
#[derive(Debug)]
pub struct AuditLog {
    file: File,
    path: PathBuf,
}

impl AuditLog {
    /// Open (creating if absent) the audit log at `path`, truncating any torn
    /// final record and positioning for append.
    ///
    /// # Errors
    ///
    /// Returns I/O errors, [`PersistError::MagicMismatch`] /
    /// [`PersistError::UnsupportedVersion`] / [`PersistError::ReservedBytesNonZero`]
    /// for a corrupt file header.
    pub fn open(path: &Path) -> PersistResult<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        let file_len = file.metadata()?.len();
        if file_len == 0 {
            write_file_header(&mut file)?;
            file.sync_all()?;
            if let Some(parent) = path.parent() {
                sync_dir(parent)?;
            }
        } else {
            verify_file_header(&mut file)?;
            let durable_end = scan_durable_end(&mut file, file_len)?;
            if durable_end < file_len {
                // Truncating the torn tail is idempotent (a re-open re-scans and
                // re-truncates), but fsync it so the reclaimed length is durable
                // immediately rather than relying on the next append's flush.
                file.set_len(durable_end)?;
                file.sync_all()?;
            }
        }
        file.seek(SeekFrom::End(0))?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }

    /// Append `record` and fsync it durable.
    ///
    /// Lifecycle events are infrequent, so each append fsyncs (data + size) —
    /// the audit trail is durable the instant the call returns. Append is the
    /// only growth path, so a crash can tear at most this final record, which
    /// [`Self::open`] discards on the next start.
    ///
    /// # Errors
    ///
    /// Returns [`PersistError::PayloadTooLarge`] if the payload exceeds
    /// [`MAX_AUDIT_PAYLOAD_BYTES`], or I/O errors from the write / fsync.
    pub fn append(&mut self, record: &AuditRecord) -> PersistResult<()> {
        let bytes = encode_record(record)?;
        self.file.write_all(&bytes)?;
        self.file.sync_data()?;
        Ok(())
    }

    /// Read every durable record in `path`, oldest first.
    ///
    /// A static reader (does not require an open [`AuditLog`]); stops at the
    /// durable tail exactly as [`Self::open`] would, so a torn trailing record
    /// is never surfaced. Returns an empty vector for an absent or
    /// header-only file.
    ///
    /// # Errors
    ///
    /// Returns I/O / header-validation errors.
    pub fn read_all(path: &Path) -> PersistResult<Vec<AuditRecord>> {
        let mut file = match OpenOptions::new().read(true).open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(PersistError::Io(error)),
        };
        let file_len = file.metadata()?.len();
        if file_len == 0 {
            return Ok(Vec::new());
        }
        verify_file_header(&mut file)?;
        read_records(&mut file, file_len)
    }

    /// Prune the log per `policy`, where `now_unix_nanos` is the reference time
    /// for [`AuditRetentionPolicy::max_age`].
    ///
    /// Reads all durable records, retains those satisfying both constraints, and
    /// atomically rewrites the log (write tmp → fsync → rename → dir fsync) so a
    /// crash mid-prune leaves the prior log intact. Records are sparse, so a full
    /// rewrite is cheap. The caller supplies `now_unix_nanos` (this crate is
    /// clock-free); pass the same epoch base used when stamping
    /// [`AuditRecord::recorded_at_unix_nanos`].
    ///
    /// # Errors
    ///
    /// Returns I/O / header errors from the read or the atomic rewrite.
    pub fn prune(
        &mut self,
        policy: &AuditRetentionPolicy,
        now_unix_nanos: u64,
    ) -> PersistResult<AuditPruneOutcome> {
        self.file.sync_data()?;
        let all = Self::read_all(&self.path)?;
        let total = all.len() as u64;
        let retained = select_retained(all, policy, now_unix_nanos);
        let retained_len = retained.len() as u64;
        if retained_len == total {
            return Ok(AuditPruneOutcome {
                retained: retained_len,
                removed: 0,
                bytes_reclaimed: 0,
            });
        }

        let size_before = self.file.metadata()?.len();
        rewrite_atomic(&self.path, &retained)?;

        // Re-open the now-rewritten file for continued appends.
        let mut file = OpenOptions::new().read(true).write(true).open(&self.path)?;
        file.seek(SeekFrom::End(0))?;
        let size_after = file.metadata()?.len();
        self.file = file;
        Ok(AuditPruneOutcome {
            retained: retained_len,
            removed: total - retained_len,
            bytes_reclaimed: size_before.saturating_sub(size_after),
        })
    }
}

/// Retain records satisfying both conjunctive constraints (newest-`keep_n`,
/// not-older-than-`max_age`), preserving oldest-first order.
fn select_retained(
    mut records: Vec<AuditRecord>,
    policy: &AuditRetentionPolicy,
    now_unix_nanos: u64,
) -> Vec<AuditRecord> {
    if let Some(max_age) = policy.max_age {
        let cutoff = now_unix_nanos.saturating_sub(nanos_from_duration(max_age));
        records.retain(|r| r.recorded_at_unix_nanos >= cutoff);
    }
    if let Some(keep_n) = policy.keep_n_events {
        let keep_n = keep_n as usize;
        if records.len() > keep_n {
            // Drop the oldest overflow (records are oldest-first).
            records.drain(0..records.len() - keep_n);
        }
    }
    records
}

/// Saturating conversion of a `Duration` to whole nanoseconds.
fn nanos_from_duration(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn write_file_header(file: &mut File) -> PersistResult<()> {
    let mut header = [0_u8; AUDIT_FILE_HEADER_LEN];
    header[0..4].copy_from_slice(&AUDIT_MAGIC);
    header[4..6].copy_from_slice(&AUDIT_FORMAT_VERSION.to_le_bytes());
    // reserved [6..8) stays zero.
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header)?;
    Ok(())
}

fn verify_file_header(file: &mut File) -> PersistResult<()> {
    let mut header = [0_u8; AUDIT_FILE_HEADER_LEN];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut header)
        .map_err(|_| PersistError::TruncatedFileHeader)?;
    let observed = [header[0], header[1], header[2], header[3]];
    if observed != AUDIT_MAGIC {
        return Err(PersistError::MagicMismatch { observed });
    }
    let version = u16::from_le_bytes([header[4], header[5]]);
    if version != AUDIT_FORMAT_VERSION {
        return Err(PersistError::UnsupportedVersion {
            major: version,
            minor: 0,
        });
    }
    if u16::from_le_bytes([header[6], header[7]]) != 0 {
        return Err(PersistError::ReservedBytesNonZero { offset: 6 });
    }
    Ok(())
}

fn encode_record(record: &AuditRecord) -> PersistResult<Vec<u8>> {
    if record.payload.len() > MAX_AUDIT_PAYLOAD_BYTES {
        return Err(PersistError::PayloadTooLarge {
            len: record.payload.len(),
            max: MAX_AUDIT_PAYLOAD_BYTES,
        });
    }
    let payload_len = record.payload.len() as u32;
    let checksum_lo = xxhash_rust::xxh3::xxh3_64(&record.payload) as u32;
    let mut out = Vec::with_capacity(AUDIT_RECORD_HEADER_LEN + record.payload.len());
    out.extend_from_slice(&record.recorded_at_unix_nanos.to_le_bytes());
    out.extend_from_slice(&record.kind.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes()); // reserved
    out.extend_from_slice(&payload_len.to_le_bytes());
    out.extend_from_slice(&checksum_lo.to_le_bytes());
    out.extend_from_slice(&record.payload);
    Ok(out)
}

/// Scan from the file header to the first torn record; return the durable end
/// offset (where the next append belongs).
fn scan_durable_end(file: &mut File, file_len: u64) -> PersistResult<u64> {
    let mut offset = AUDIT_FILE_HEADER_LEN as u64;
    loop {
        match read_one_record(file, offset, file_len)? {
            Some((_, next)) => offset = next,
            None => return Ok(offset),
        }
    }
}

fn read_records(file: &mut File, file_len: u64) -> PersistResult<Vec<AuditRecord>> {
    let mut out = Vec::new();
    let mut offset = AUDIT_FILE_HEADER_LEN as u64;
    while let Some((record, next)) = read_one_record(file, offset, file_len)? {
        out.push(record);
        offset = next;
    }
    Ok(out)
}

/// Read the record at `offset`. Returns `Ok(Some((record, next_offset)))` for a
/// good record, or `Ok(None)` at a clean end or the first torn record (short
/// read, over-cap length, or checksum mismatch — all treated as the durable
/// tail, never a hard error, mirroring the WAL scan).
fn read_one_record(
    file: &mut File,
    offset: u64,
    file_len: u64,
) -> PersistResult<Option<(AuditRecord, u64)>> {
    if offset >= file_len {
        return Ok(None);
    }
    if file_len - offset < AUDIT_RECORD_HEADER_LEN as u64 {
        return Ok(None); // torn header
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut header = [0_u8; AUDIT_RECORD_HEADER_LEN];
    if file.read_exact(&mut header).is_err() {
        return Ok(None);
    }
    let recorded_at_unix_nanos = u64::from_le_bytes(header[0..8].try_into().expect("8 bytes"));
    let kind = u16::from_le_bytes([header[8], header[9]]);
    // header[10..12) reserved — tolerated on read (forward-compat).
    let payload_len = u32::from_le_bytes(header[12..16].try_into().expect("4 bytes")) as usize;
    let checksum_lo = u32::from_le_bytes(header[16..20].try_into().expect("4 bytes"));

    if payload_len > MAX_AUDIT_PAYLOAD_BYTES {
        return Ok(None); // garbage length: treat as torn tail
    }
    let payload_start = offset + AUDIT_RECORD_HEADER_LEN as u64;
    let payload_end = payload_start + payload_len as u64;
    if payload_end > file_len {
        return Ok(None); // torn payload
    }
    let mut payload = vec![0_u8; payload_len];
    if file.read_exact(&mut payload).is_err() {
        return Ok(None);
    }
    if xxhash_rust::xxh3::xxh3_64(&payload) as u32 != checksum_lo {
        return Ok(None); // checksum mismatch: torn / corrupt tail
    }
    Ok(Some((
        AuditRecord {
            recorded_at_unix_nanos,
            kind,
            payload,
        },
        payload_end,
    )))
}

/// Atomically rewrite `path` to hold exactly `records` (file header + each
/// record), via write-tmp → fsync → rename → dir fsync.
fn rewrite_atomic(path: &Path, records: &[AuditRecord]) -> PersistResult<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp_path = path.with_extension("log.tmp");

    let result = (|| -> PersistResult<()> {
        let mut tmp = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)?;
        write_file_header(&mut tmp)?;
        tmp.seek(SeekFrom::End(0))?;
        for record in records {
            tmp.write_all(&encode_record(record)?)?;
        }
        tmp.sync_all()?;
        drop(tmp);
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
        return result;
    }
    sync_dir(dir)
}

#[cfg(test)]
mod tests;
