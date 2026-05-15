//! Append-only WAL writer.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

use selene_core::{Change, HlcTimestamp, Origin};

use crate::entry_header::{
    encode_entry_header, ensure_payload_len, read_entry_header, validate_principal,
};
use crate::file_header::{WAL_FILE_HEADER_LEN, WalFileHeader};
use crate::payload::{encode_changes, verify_checksum};
use crate::{PersistError, PersistResult, WalEntryHeader};

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
    last_sequence: u64,
    sync_policy: SyncPolicy,
    entries_since_fsync: u32,
    /// File offset of the last fully-committed entry's end. On any
    /// append-time error, the file is truncated and re-seeked to this
    /// offset so the writer's in-memory state and the on-disk state stay
    /// consistent.
    committed_offset: u64,
}

impl WalWriter {
    /// Open a WAL file for append, creating the v1 header for a new file.
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
        let sync_policy = config.sync_policy.normalized();
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
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
            last_sequence,
            sync_policy,
            entries_since_fsync: 0,
            committed_offset: scan.truncate_to,
        })
    }

    /// Append one WAL entry and return its assigned sequence.
    ///
    /// # Errors
    ///
    /// Returns codec, cap, compression, or I/O errors. On any error, the
    /// in-memory sequence counter is **not** advanced and the file is
    /// truncated back to the last fully-committed entry, so the next
    /// append (or a reopen + retry) observes a consistent state.
    pub fn append(
        &mut self,
        hlc: HlcTimestamp,
        origin: Origin,
        principal: Option<Arc<[u8]>>,
        changes: &[Change],
    ) -> PersistResult<u64> {
        validate_principal(principal.as_deref())?;
        let payload = encode_changes(changes)?;
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
        let mut record = Vec::with_capacity(header_bytes.len() + payload.bytes.len());
        record.extend_from_slice(&header_bytes);
        record.extend_from_slice(&payload.bytes);

        let result = (|| -> PersistResult<()> {
            self.file.write_all(&record)?;
            if needs_fsync {
                self.file.sync_data()?;
            }
            Ok(())
        })();

        match result {
            Ok(()) => {
                let new_offset = self.committed_offset.saturating_add(record.len() as u64);
                self.committed_offset = new_offset;
                self.last_sequence = sequence;
                self.entries_since_fsync = if needs_fsync { 0 } else { pending_count };
                Ok(sequence)
            }
            Err(error) => {
                self.rollback_to_committed_offset();
                Err(error)
            }
        }
    }

    /// Flush + fsync without appending. Useful before snapshot publication.
    ///
    /// # Errors
    ///
    /// Returns I/O errors from fsync.
    pub fn flush(&mut self) -> PersistResult<()> {
        self.file.sync_data()?;
        self.entries_since_fsync = 0;
        Ok(())
    }

    /// Return the last sequence assigned by this writer.
    #[must_use]
    pub const fn last_sequence(&self) -> u64 {
        self.last_sequence
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

        file.seek(SeekFrom::Start(offset))?;
        let header = match read_entry_header(&mut *file, offset) {
            Ok((header, _)) => header,
            Err(PersistError::TruncatedEntry { .. }) | Err(PersistError::HeaderCodec(_)) => {
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
        let payload_start = file.stream_position()?;
        let payload_end = payload_start.saturating_add(u64::from(header.payload_len));
        if payload_end > file_len {
            return Ok(Scan {
                last_sequence: previous,
                truncate_to: last_valid_offset,
            });
        }
        let mut payload = vec![0_u8; payload_len];
        file.read_exact(&mut payload)?;
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
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use selene_core::{Change, NodeId, Origin, PropertyMap, intern};

    use super::*;
    use crate::{MAX_PRINCIPAL_BYTES, WAL_FILE_HEADER_LEN, WalReader};

    fn temp_path(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "selene-persist-{name}-{}-{nanos}.wal",
            std::process::id()
        ))
    }

    fn changes() -> Vec<Change> {
        vec![Change::NodeCreated {
            id: NodeId::new(1),
            labels: selene_core::LabelSet::single(intern("writer.node").unwrap()),
            properties: PropertyMap::new(),
        }]
    }

    #[test]
    fn open_new_file_writes_header() {
        let path = temp_path("open-new");
        {
            let writer = WalWriter::open(&path, WalConfig::default()).unwrap();
            assert_eq!(writer.last_sequence(), 0);
        }
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            WAL_FILE_HEADER_LEN as u64
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn append_assigns_monotonic_sequences() {
        let path = temp_path("seq");
        let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
        assert_eq!(
            writer
                .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
                .unwrap(),
            1
        );
        assert_eq!(
            writer
                .append(HlcTimestamp::new(1, 1), Origin::Local, None, &changes())
                .unwrap(),
            2
        );
        assert_eq!(writer.last_sequence(), 2);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reopen_recovers_last_sequence() {
        let path = temp_path("reopen");
        {
            let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
            writer
                .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
                .unwrap();
        }
        let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
        assert_eq!(writer.last_sequence(), 1);
        assert_eq!(
            writer
                .append(HlcTimestamp::new(2, 0), Origin::Local, None, &changes())
                .unwrap(),
            2
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn principal_overflow_does_not_increment_sequence() {
        let path = temp_path("principal");
        let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
        let len_before = fs::metadata(&path).unwrap().len();
        let err = writer
            .append(
                HlcTimestamp::zero(),
                Origin::Local,
                Some(Arc::from(vec![1_u8; MAX_PRINCIPAL_BYTES + 1])),
                &changes(),
            )
            .unwrap_err();
        assert!(matches!(err, PersistError::PrincipalTooLarge { .. }));
        assert_eq!(writer.last_sequence(), 0);
        assert_eq!(fs::metadata(&path).unwrap().len(), len_before);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn partial_tail_is_truncated_on_open() {
        let path = temp_path("tail");
        {
            let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
            writer
                .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
                .unwrap();
            writer.flush().unwrap();
        }
        let valid_len = fs::metadata(&path).unwrap().len();
        {
            let mut file = OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(&[0, 1, 2]).unwrap();
        }
        let writer = WalWriter::open(&path, WalConfig::default()).unwrap();
        assert_eq!(writer.last_sequence(), 1);
        assert_eq!(fs::metadata(&path).unwrap().len(), valid_len);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn group_commit_defers_fsync_until_threshold() {
        let path = temp_path("group");
        let mut writer = WalWriter::open(
            &path,
            WalConfig {
                sync_policy: SyncPolicy::EveryN(3),
                snapshot_seq: 0,
            },
        )
        .unwrap();
        assert_eq!(writer.sync_policy.as_every_n(), Some(3));
        writer
            .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
            .unwrap();
        writer
            .append(HlcTimestamp::new(1, 1), Origin::Local, None, &changes())
            .unwrap();
        assert_eq!(writer.entries_since_fsync, 2);
        writer
            .append(HlcTimestamp::new(1, 2), Origin::Local, None, &changes())
            .unwrap();
        assert_eq!(writer.entries_since_fsync, 0);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn wal_config_default_is_every_n_1() {
        assert_eq!(WalConfig::default().sync_policy, SyncPolicy::EveryN(1));
    }

    #[test]
    fn with_fsync_every_n_preserves_legacy_threshold() {
        assert_eq!(
            WalConfig::with_fsync_every_n(7),
            WalConfig {
                sync_policy: SyncPolicy::EveryN(7),
                snapshot_seq: 0,
            }
        );
    }

    #[test]
    fn every_n_zero_normalizes_to_one_on_open() {
        let path = temp_path("normalize");
        let writer = WalWriter::open(
            &path,
            WalConfig {
                sync_policy: SyncPolicy::EveryN(0),
                snapshot_seq: 0,
            },
        )
        .unwrap();
        assert_eq!(writer.sync_policy.as_every_n(), Some(1));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn on_flush_only_accumulates_until_explicit_flush() {
        let path = temp_path("on-flush");
        let mut writer = WalWriter::open(
            &path,
            WalConfig {
                sync_policy: SyncPolicy::OnFlushOnly,
                snapshot_seq: 0,
            },
        )
        .unwrap();
        for tick in 0..3 {
            writer
                .append(HlcTimestamp::new(1, tick), Origin::Local, None, &changes())
                .unwrap();
        }
        assert_eq!(writer.entries_since_fsync, 3);
        writer.flush().unwrap();
        assert_eq!(writer.entries_since_fsync, 0);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn on_flush_only_drop_does_not_fsync() {
        assert!(!SyncPolicy::OnFlushOnly.syncs_on_drop());
        assert!(SyncPolicy::EveryN(1).syncs_on_drop());
    }

    #[test]
    fn replicated_origin_round_trips_node_id_and_source_seq() {
        let path = temp_path("origin");
        {
            let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
            writer
                .append(
                    HlcTimestamp::new(1, 0),
                    Origin::Replicated {
                        source_node_id: NodeId::new(77),
                        source_seq: 9,
                    },
                    None,
                    &changes(),
                )
                .unwrap();
        }
        let reader = WalReader::open(&path).unwrap();
        let entry = reader.iterate(|_| true).unwrap().next().unwrap().unwrap();
        assert_eq!(
            entry.header.origin,
            Origin::Replicated {
                source_node_id: NodeId::new(77),
                source_seq: 9,
            }
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn snapshot_seq_seeds_first_appended_sequence() {
        let path = temp_path("snapshot-seq-seed");
        let mut writer = WalWriter::open(
            &path,
            WalConfig {
                sync_policy: SyncPolicy::EveryN(1),
                snapshot_seq: 100,
            },
        )
        .unwrap();
        // First append must follow snapshot_seq + 1 = 101, not start at 1.
        let seq = writer
            .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
            .unwrap();
        assert_eq!(seq, 101);
        assert_eq!(writer.last_sequence(), 101);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reopen_uses_header_snapshot_seq_not_config() {
        let path = temp_path("reopen-snapshot-seq");
        // Create with snapshot_seq=100
        {
            let mut writer = WalWriter::open(
                &path,
                WalConfig {
                    sync_policy: SyncPolicy::EveryN(1),
                    snapshot_seq: 100,
                },
            )
            .unwrap();
            assert_eq!(writer.last_sequence(), 100);
            writer
                .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
                .unwrap();
        }
        // Reopen with a stale config.snapshot_seq=0 — header wins, last
        // appended sequence (101) is recovered.
        let writer = WalWriter::open(
            &path,
            WalConfig {
                sync_policy: SyncPolicy::EveryN(1),
                snapshot_seq: 0,
            },
        )
        .unwrap();
        assert_eq!(writer.last_sequence(), 101);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn second_writer_open_returns_writer_lock_held() {
        let path = temp_path("lock");
        let _first = WalWriter::open(&path, WalConfig::default()).unwrap();
        let second = WalWriter::open(&path, WalConfig::default());
        assert!(matches!(second, Err(PersistError::WriterLockHeld)));
        drop(_first);
        // After the first writer drops, a fresh open must succeed (the
        // OS releases the file lock when the File handle closes).
        let third = WalWriter::open(&path, WalConfig::default());
        assert!(third.is_ok());
        drop(third);
        let _ = fs::remove_file(path);
    }
}
