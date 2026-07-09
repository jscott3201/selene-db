//! Append-only WAL writer.

use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use selene_core::{Change, HlcTimestamp, Origin, metrics};

use crate::compression::ZstdCompressor;
use crate::entry_header::{
    encode_entry_header, ensure_payload_len, read_entry_header, validate_principal,
};
use crate::file_header::{WAL_FILE_HEADER_LEN, WalFileHeader};
use crate::manifest::Manifest;
use crate::payload::{WalCompression, encode_changes_with_compressor, verify_checksum};
use crate::retention::{PruneOutcome, RetentionPolicy};
use crate::snapshot_writer::SnapshotBuilder;
use crate::writer_rotation::{RotationInputs, WalRotationOutcome, rotate_with_manifest};
use crate::{PersistError, PersistResult, WalEntryHeader};

const WAL_RECORD_BUFFER_RETAIN_LIMIT: usize = 4 * 1024 * 1024;

/// Conventional v1.0 single-file WAL name used by embedders.
pub const DEFAULT_WAL_FILE_NAME: &str = "wal.log";

/// WAL fsync scheduling policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncPolicy {
    /// Flush and fsync after every `N` appended entries, on explicit flush,
    /// and when the writer is dropped.
    ///
    /// `EveryN(1)` is durability-by-default. Values greater than `1` opt into
    /// group commit. `EveryN(0)` is normalized to `EveryN(1)` on open.
    EveryN(u32),
    /// Never fsync during append or drop; only explicit [`WalWriter::flush`]
    /// fsyncs.
    ///
    /// This is an explicit opt-in for benchmark parity and offline paths where
    /// durability is provided elsewhere. It is not the production default.
    ///
    /// # selene-graph forces this for the committer WAL (v1.2 BRIEF 2)
    ///
    /// When a [`WalWriter`] is owned by selene-graph's single committer thread
    /// (via `SharedGraphBuilder::with_wal` / `SharedGraph::from_graph_with_wal` /
    /// recovery), the committer is the **sole fsync caller**: it appends a
    /// contiguous run of commits with fsync deferred, then issues exactly one
    /// [`WalWriter::flush`] per run as the fsync-before-publish barrier. To make
    /// that the only fsync path, selene-graph **overrides `WalConfig::sync_policy`
    /// to `OnFlushOnly`** before opening such a WAL, discarding any caller policy
    /// (the fsync cadence is instead set by `selene_graph::CommitBatching`). A
    /// `WalWriter` opened directly (outside selene-graph) still honors whatever
    /// policy the caller passes — the override lives in selene-graph, not here.
    OnFlushOnly,
}

impl SyncPolicy {
    /// Return the `EveryN` threshold when this policy syncs on append.
    #[must_use]
    pub const fn as_every_n(self) -> Option<u32> {
        match self {
            Self::EveryN(value) => Some(value),
            Self::OnFlushOnly => None,
        }
    }

    const fn normalized(self) -> Self {
        match self {
            Self::EveryN(0) => Self::EveryN(1),
            policy => policy,
        }
    }

    const fn syncs_on_drop(self) -> bool {
        matches!(self, Self::EveryN(_))
    }
}

/// WAL writer configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WalConfig {
    /// Flush and fsync schedule.
    pub sync_policy: SyncPolicy,
    /// Highest WAL sequence covered by the snapshot this file extends.
    ///
    /// Written into the file header on a fresh file, and used to seed
    /// `last_sequence` so the first appended entry receives sequence
    /// `snapshot_seq + 1`. On reopen, the on-disk header is the source of
    /// truth and the config value is ignored — recovery never moves a
    /// snapshot watermark backward.
    pub snapshot_seq: u64,
}

impl Default for WalConfig {
    fn default() -> Self {
        Self {
            sync_policy: SyncPolicy::EveryN(1),
            snapshot_seq: 0,
        }
    }
}

impl WalConfig {
    /// Construct a WAL config with the legacy group-commit threshold.
    #[must_use]
    pub const fn with_fsync_every_n(fsync_every_n: u32) -> Self {
        Self {
            sync_policy: SyncPolicy::EveryN(fsync_every_n),
            snapshot_seq: 0,
        }
    }
}

/// Single-threaded append-only WAL writer.
///
/// Holds an exclusive OS-level file lock on the WAL file for the writer's
/// lifetime, so a second `WalWriter::open` call on the same path
/// (in-process or cross-process) fails fast with
/// [`PersistError::WriterLockHeld`] rather than corrupting the log.
pub struct WalWriter {
    file: File,
    path: PathBuf,
    record: Vec<u8>,
    last_sequence: u64,
    snapshot_seq: u64,
    sync_policy: SyncPolicy,
    compression: WalCompression,
    compressor: Option<ZstdCompressor>,
    entries_since_fsync: u32,
    poisoned: bool,
    /// File offset of the last fully-committed entry's end. On any
    /// append-time error, the file is truncated and re-seeked to this
    /// offset so the writer's in-memory state and the on-disk state stay
    /// consistent.
    committed_offset: u64,
}

impl WalWriter {
    /// Open a WAL file for append, creating the v2 header for a new file.
    ///
    /// Existing files are scanned once to find the last valid entry. A partial
    /// or checksum-invalid tail is truncated to the last valid offset.
    ///
    /// Acquires an exclusive OS-level file lock; a second writer on the
    /// same path fails immediately with
    /// [`PersistError::WriterLockHeld`] instead of clobbering the log.
    ///
    /// # Errors
    ///
    /// Returns I/O, header, sequence, lock, or checksum errors encountered
    /// while opening and validating the WAL.
    pub fn open(path: &Path, config: WalConfig) -> PersistResult<Self> {
        Self::open_with_compression(path, config, WalCompression::default())
    }

    /// Open a WAL file with an explicit payload compression policy.
    ///
    /// This keeps the same file format as [`Self::open`]; only the append-time
    /// decision to compress each serialized payload changes. Existing readers
    /// continue to use the per-entry compression flag stored in the header.
    ///
    /// # Errors
    ///
    /// Returns I/O, header, sequence, lock, or checksum errors encountered
    /// while opening and validating the WAL.
    pub fn open_with_compression(
        path: &Path,
        config: WalConfig,
        compression: WalCompression,
    ) -> PersistResult<Self> {
        let sync_policy = config.sync_policy.normalized();
        // Retain a CWD-independent path for every later snapshot/archive/
        // MANIFEST operation. The file handle stays bound to the inode opened
        // here; keeping a caller-relative path would let a later `set_current_dir`
        // redirect rotation artifacts away from the active file this handle owns.
        let stable_path = stable_wal_path(path)?;
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&stable_path)?;
        // Acquire an exclusive lock before doing anything else. A second
        // writer on the same path observes WriterLockHeld and returns
        // without touching the file.
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(PersistError::WriterLockHeld);
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        let len = file.metadata()?.len();
        let header_snapshot_seq = if len == 0 {
            WalFileHeader::new(config.snapshot_seq).write_to(&mut file)?;
            file.sync_data()?;
            config.snapshot_seq
        } else {
            file.seek(SeekFrom::Start(0))?;
            WalFileHeader::read_from(&mut file)?.snapshot_seq
        };

        let scan = scan_existing(&mut file)?;
        if scan.truncate_to < file.metadata()?.len() {
            tracing::warn!(
                offset = scan.truncate_to,
                "truncating WAL tail to last valid entry"
            );
            file.set_len(scan.truncate_to)?;
        }
        file.seek(SeekFrom::Start(scan.truncate_to))?;

        // Seed last_sequence from the larger of (header watermark, last
        // scanned entry). On a fresh file, scan returns 0 and the
        // watermark wins. On reopen with entries that already extend past
        // the snapshot, the entry sequence wins.
        let last_sequence = scan.last_sequence.max(header_snapshot_seq);
        Ok(Self {
            file,
            path: stable_path,
            record: Vec::new(),
            last_sequence,
            snapshot_seq: header_snapshot_seq,
            sync_policy,
            compression,
            compressor: None,
            entries_since_fsync: 0,
            poisoned: false,
            committed_offset: scan.truncate_to,
        })
    }

    /// Append one WAL entry and return its assigned sequence.
    ///
    /// # Errors
    ///
    /// Returns [`PersistError::WalWriterPoisoned`] after an incomplete rotation,
    /// or codec, cap, compression, or I/O errors. On any error, the in-memory
    /// sequence counter is **not** advanced and the file is
    /// truncated back to the last fully-committed entry, so the next
    /// append (or a reopen + retry) observes a consistent state.
    #[tracing::instrument(
        name = "selene.persist.wal.append",
        skip(self, principal, changes),
        fields(sequence = self.last_sequence + 1, change_count = changes.len(), has_principal = principal.is_some())
    )]
    pub fn append(
        &mut self,
        hlc: HlcTimestamp,
        origin: Origin,
        principal: Option<Arc<[u8]>>,
        changes: &[Change],
    ) -> PersistResult<u64> {
        self.ensure_usable()?;
        validate_principal(principal.as_deref())?;
        let payload =
            encode_changes_with_compressor(changes, self.compression, Some(&mut self.compressor))?;
        let sequence = self.last_sequence + 1;
        let header = WalEntryHeader::new(
            payload.bytes.len(),
            payload.checksum_lo,
            sequence,
            hlc,
            origin,
            payload.flags,
            principal,
        )?;
        let header_bytes = encode_entry_header(&header)?;
        let pending_count = self.entries_since_fsync.saturating_add(1);
        let needs_fsync = match self.sync_policy {
            SyncPolicy::EveryN(threshold) => pending_count >= threshold,
            SyncPolicy::OnFlushOnly => false,
        };

        // Single contiguous record. Write it in one syscall via a Vec
        // assembly so partial writes are easier to reason about.
        self.record.clear();
        self.record
            .reserve(header_bytes.len() + payload.bytes.len());
        self.record.extend_from_slice(&header_bytes);
        self.record.extend_from_slice(&payload.bytes);
        let record_len = self.record.len();

        let result = (|| -> PersistResult<()> {
            self.file.write_all(&self.record)?;
            if needs_fsync {
                self.file.sync_data()?;
            }
            Ok(())
        })();

        match result {
            Ok(()) => {
                let new_offset = self.committed_offset.saturating_add(record_len as u64);
                self.committed_offset = new_offset;
                self.last_sequence = sequence;
                self.entries_since_fsync = if needs_fsync { 0 } else { pending_count };
                metrics::counter_inc(metrics::WAL_APPENDS_TOTAL);
                self.trim_record_buffer();
                Ok(sequence)
            }
            Err(error) => {
                self.rollback_to_committed_offset();
                self.trim_record_buffer();
                Err(error)
            }
        }
    }

    fn trim_record_buffer(&mut self) {
        if self.record.capacity() > WAL_RECORD_BUFFER_RETAIN_LIMIT {
            self.record = Vec::new();
        }
    }

    /// Flush + fsync without appending. Useful before snapshot publication.
    ///
    /// # Errors
    ///
    /// Returns [`PersistError::WalWriterPoisoned`] after an incomplete rotation,
    /// or I/O errors from fsync.
    #[tracing::instrument(name = "selene.persist.wal.fsync", skip(self))]
    pub fn flush(&mut self) -> PersistResult<()> {
        self.ensure_usable()?;
        self.file.sync_data()?;
        self.entries_since_fsync = 0;
        Ok(())
    }

    /// Return the last sequence assigned by this writer.
    #[must_use]
    pub const fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    /// Return the path of the active WAL file owned by this writer.
    ///
    /// Relative caller paths are resolved against the working directory at
    /// [`Self::open`] time, so this path remains stable if the process later
    /// changes its working directory. It is exposed read-only so graph-layer
    /// checkpoint orchestration can verify the conventional `wal.log` layout
    /// before committing a MANIFEST rotation.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the durable file offset of the last fully committed WAL entry.
    #[must_use]
    pub const fn committed_offset(&self) -> u64 {
        self.committed_offset
    }

    /// Return the snapshot sequence stored in this WAL file's header.
    #[must_use]
    pub const fn snapshot_seq(&self) -> u64 {
        self.snapshot_seq
    }

    /// Return the number of entries appended since the last fsync.
    #[must_use]
    pub const fn entries_since_fsync(&self) -> u32 {
        self.entries_since_fsync
    }

    /// Crash-safe rotate: finalize `builder`, commit a MANIFEST, then reset.
    ///
    /// This is the v1.x replacement for the embedder's two-call
    /// finalize-then-`rotate` sequence. It runs the 4-phase rotation
    /// whose MANIFEST write is the single linearization / commit point, so a
    /// crash at any point either leaves the previous epoch fully recoverable or
    /// the new epoch fully committed — never the [`PersistError::WalSnapshotMismatch`]
    /// (Seam F) hard-fail the split calls could produce.
    ///
    /// `builder` must target the same sequence as this writer's current
    /// high-water mark (`builder.sequence() == self.last_sequence()`); the
    /// builder is finalized as Phase 1, so the caller adds every section before
    /// calling. The MANIFEST's `archived_wal_seqs` extends the set already named
    /// by any live MANIFEST in this writer's directory, so retention (Item-5)
    /// has the full archive history.
    ///
    /// Before the first Phase 1 in a legacy MANIFEST-less directory, rotation
    /// durably bootstraps a baseline MANIFEST naming the current WAL-header
    /// snapshot epoch. A non-zero baseline must already have a valid snapshot;
    /// otherwise rotation rejects it before publishing the requested snapshot.
    /// Phase 4 uses a synced same-directory replacement + atomic rename, so a
    /// reset failure leaves recovery with either the old valid WAL or the new
    /// valid header rather than a truncated active file.
    /// Rotation accepts only the conventional `wal.log` filename, a positive
    /// sequence, a builder directory resolving to the WAL's stable parent, and
    /// an existing MANIFEST that also names `wal.log` as its active WAL.
    ///
    /// A second mutable borrow cannot overlap the rotation:
    ///
    /// ```compile_fail
    /// # use selene_persist::{SnapshotBuilder, SnapshotConfig, WalWriter};
    /// fn cannot_overlap(writer: &mut WalWriter, builder: SnapshotBuilder) {
    ///     let active = writer;
    ///     let _ = writer.rotate_with_manifest(builder);
    ///     let _ = active.last_sequence();
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns typed errors for a poisoned writer, a non-default WAL filename,
    /// sequence zero, a builder/WAL directory mismatch, or a builder sequence
    /// that does not match the writer high-water mark; I/O / format errors from
    /// snapshot finalize, archive, MANIFEST commit, or WAL reset; or
    /// [`PersistError::WalRotationIncomplete`] if the MANIFEST committed but the
    /// active WAL could not be reset (recovery still converges on the new
    /// epoch). After `WalRotationIncomplete`, this writer is poisoned and
    /// rejects append, flush, rotation, and prune calls; reopen the active WAL
    /// before mutating again. On error before the MANIFEST commit the previous
    /// epoch is intact and the writer remains usable.
    pub fn rotate_with_manifest(
        &mut self,
        builder: SnapshotBuilder,
    ) -> PersistResult<WalRotationOutcome> {
        self.ensure_usable()?;
        let dir = self.validate_rotation_inputs(&builder)?;
        let prior_manifest = Manifest::read(&dir)?;
        if let Some(manifest) = &prior_manifest
            && manifest.active_wal != DEFAULT_WAL_FILE_NAME
        {
            return Err(PersistError::UnexpectedActiveWal {
                observed: manifest.active_wal.clone(),
                expected: DEFAULT_WAL_FILE_NAME,
            });
        }
        // Every input invariant above is checked before this durability write.
        // Invalid callers must not flush group-commit state or create artifacts.
        self.flush()?;
        let manifest_present = prior_manifest.is_some();
        let prior_archived_seqs = prior_manifest
            .map(|manifest| manifest.archived_wal_seqs)
            .unwrap_or_default();
        let inputs = RotationInputs {
            file: &mut self.file,
            wal_path: &self.path,
            committed_offset: self.committed_offset,
            last_sequence: self.last_sequence,
            snapshot_seq: self.snapshot_seq,
            manifest_present,
            prior_archived_seqs,
        };
        let (outcome, state) = match rotate_with_manifest(inputs, builder, &dir) {
            Ok(result) => result,
            Err(error @ PersistError::WalRotationIncomplete { .. }) => {
                self.poisoned = true;
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        self.last_sequence = state.last_sequence;
        self.snapshot_seq = state.snapshot_seq;
        self.committed_offset = state.committed_offset;
        self.entries_since_fsync = 0;
        Ok(outcome)
    }

    fn validate_rotation_inputs(&self, builder: &SnapshotBuilder) -> PersistResult<PathBuf> {
        let observed_name = self
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string());
        if observed_name != DEFAULT_WAL_FILE_NAME {
            return Err(PersistError::UnexpectedActiveWal {
                observed: observed_name,
                expected: DEFAULT_WAL_FILE_NAME,
            });
        }
        if builder.sequence() == 0 || self.last_sequence == 0 {
            return Err(PersistError::WalRotationZeroSequence);
        }
        if builder.sequence() != self.last_sequence {
            return Err(PersistError::WalRotationSequenceMismatch {
                snapshot_seq: builder.sequence(),
                last_sequence: self.last_sequence,
            });
        }
        let wal_dir = self
            .path
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let snapshot_dir = stable_wal_path(builder.dir())?;
        let canonical_snapshot_dir = std::fs::canonicalize(&snapshot_dir)?;
        let canonical_wal_dir = std::fs::canonicalize(&wal_dir)?;
        if canonical_snapshot_dir != canonical_wal_dir {
            return Err(PersistError::WalRotationDirectoryMismatch {
                snapshot_dir,
                wal_dir,
            });
        }
        Ok(wal_dir)
    }

    /// Prune superseded snapshots + WAL archives in this writer's directory per
    /// `policy`, committing through the MANIFEST.
    ///
    /// Thin ergonomic wrapper over [`crate::retention::prune`] bound to this
    /// writer's directory. Pending appends are flushed first so the on-disk
    /// state the prune reasons about is current, and the `&mut self` receiver
    /// serializes the prune against [`Self::rotate_with_manifest`] — the two
    /// must never interleave their MANIFEST rewrites. The prune never touches
    /// the active WAL this writer owns; it only reclaims snapshot/archive files
    /// the live epoch no longer needs.
    ///
    /// # Errors
    ///
    /// Returns [`PersistError::WalWriterPoisoned`] after an incomplete rotation,
    /// flush errors, or any error from [`crate::retention::prune`] (directory
    /// scan, MANIFEST decode/commit). Post-commit file deletion is best-effort
    /// and never fails the prune.
    pub fn prune(&mut self, policy: &RetentionPolicy) -> PersistResult<PruneOutcome> {
        self.ensure_usable()?;
        self.flush()?;
        let dir = self
            .path
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        crate::retention::prune(&dir, policy)
    }

    fn ensure_usable(&self) -> PersistResult<()> {
        if self.poisoned {
            Err(PersistError::WalWriterPoisoned)
        } else {
            Ok(())
        }
    }

    /// Best-effort rollback to the last committed offset on append failure.
    /// On rollback failure, the writer is left in a half-consistent state;
    /// the caller should reopen the WAL (which scan-truncates on open) to
    /// recover.
    fn rollback_to_committed_offset(&mut self) {
        if let Err(error) = self.file.set_len(self.committed_offset) {
            tracing::error!(%error, "failed to truncate WAL after append error");
            return;
        }
        if let Err(error) = self.file.seek(SeekFrom::Start(self.committed_offset)) {
            tracing::error!(%error, "failed to seek WAL after append error");
        }
    }
}

fn stable_wal_path(path: &Path) -> std::io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

impl Drop for WalWriter {
    fn drop(&mut self) {
        if self.sync_policy.syncs_on_drop()
            && let Err(error) = self.file.sync_data()
        {
            tracing::error!(%error, "failed to fsync WAL writer on drop");
        }
        // The exclusive file lock is released when `file` is dropped.
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Scan {
    last_sequence: u64,
    truncate_to: u64,
}

fn scan_existing(file: &mut File) -> PersistResult<Scan> {
    let file_len = file.metadata()?.len();
    let mut offset = WAL_FILE_HEADER_LEN as u64;
    let mut previous = 0_u64;
    let mut last_valid_offset = offset;
    let mut payload = Vec::new();
    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);

    loop {
        if offset == file_len {
            return Ok(Scan {
                last_sequence: previous,
                truncate_to: last_valid_offset,
            });
        }
        if offset > file_len {
            return Ok(Scan {
                last_sequence: previous,
                truncate_to: last_valid_offset,
            });
        }

        let (header, bytes_consumed) = match read_entry_header(&mut reader, offset) {
            Ok(header) => header,
            // Treat oversized decoded headers as torn-tail too: with the
            // fixed-layout v2 format, garbage bytes can decode as a valid
            // u32 payload_len > MAX_WAL_ENTRY_BYTES or u16 principal_len > cap.
            // Pre-v2 (postcard varint), oversized lengths surfaced as
            // HeaderCodec; v2 surfaces them as typed cap errors that must be
            // routed through the same recovery path.
            Err(PersistError::TruncatedEntry { .. })
            | Err(PersistError::HeaderCodec(_))
            | Err(PersistError::PayloadTooLarge { .. })
            | Err(PersistError::PrincipalTooLarge { .. }) => {
                return Ok(Scan {
                    last_sequence: previous,
                    truncate_to: last_valid_offset,
                });
            }
            Err(error) => return Err(error),
        };

        if header.sequence <= previous {
            return Err(PersistError::NonMonotonicSequence {
                previous,
                current: header.sequence,
            });
        }
        let payload_len = header.payload_len as usize;
        if ensure_payload_len(payload_len).is_err() {
            return Ok(Scan {
                last_sequence: previous,
                truncate_to: last_valid_offset,
            });
        }
        let payload_start = offset.saturating_add(bytes_consumed as u64);
        let payload_end = payload_start.saturating_add(u64::from(header.payload_len));
        if payload_end > file_len {
            return Ok(Scan {
                last_sequence: previous,
                truncate_to: last_valid_offset,
            });
        }
        payload.resize(payload_len, 0);
        reader.read_exact(&mut payload)?;
        if verify_checksum(&header, &payload).is_err() {
            return Ok(Scan {
                last_sequence: previous,
                truncate_to: last_valid_offset,
            });
        }
        previous = header.sequence;
        offset = payload_end;
        last_valid_offset = payload_end;
    }
}

#[cfg(test)]
mod tests;
