//! WAL rotation helpers, including the crash-safe multi-phase rotate
//! orchestrator whose MANIFEST commit is the rotation linearization point.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::file_header::{WAL_FILE_HEADER_LEN, WalFileHeader};
use crate::manifest::{Manifest, sync_dir};
use crate::snapshot_path::snapshot_path;
use crate::snapshot_writer::SnapshotBuilder;
use crate::{DEFAULT_WAL_FILE_NAME, PersistError, PersistResult, SnapshotReader};

/// Result of a successful WAL rotation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WalRotationOutcome {
    /// Archive path containing the pre-rotation WAL.
    pub archived_path: PathBuf,
    /// Active WAL path reopened for post-snapshot appends.
    pub new_path: PathBuf,
    /// Last sequence contained by the archived WAL.
    pub archived_last_sequence: u64,
}

/// Filename prefix shared by every archived WAL segment (`wal.{seq}.archive`).
pub const WAL_ARCHIVE_PREFIX: &str = "wal.";
/// Filename suffix shared by every archived WAL segment (`wal.{seq}.archive`).
pub const WAL_ARCHIVE_SUFFIX: &str = ".archive";

pub(crate) fn wal_archive_path(path: &Path, last_sequence: u64) -> PathBuf {
    let archive_name = format!("{WAL_ARCHIVE_PREFIX}{last_sequence}{WAL_ARCHIVE_SUFFIX}");
    path.parent().map_or_else(
        || PathBuf::from(&archive_name),
        |parent| parent.join(&archive_name),
    )
}

/// Parse `wal.{seq}.archive` into its sequence number.
///
/// Returns `None` for any name that does not match the durable-archive pattern,
/// including the in-flight `wal.{seq}.archive.tmp.{pid}.{n}` temporaries a
/// crashed rotation may leave behind (their trailing component is the attempt
/// counter, not `.archive`). Retention's directory scan relies on this so it
/// never mistakes a partial archive for a prunable durable one — the mirror of
/// [`crate::snapshot_path::parse_snapshot_filename`] for the WAL archive family.
#[must_use]
pub fn parse_wal_archive_filename(name: &std::ffi::OsStr) -> Option<u64> {
    let name = name.to_str()?;
    let seq = name
        .strip_prefix(WAL_ARCHIVE_PREFIX)?
        .strip_suffix(WAL_ARCHIVE_SUFFIX)?;
    if seq.is_empty() {
        return None;
    }
    seq.parse::<u64>().ok()
}

pub(crate) fn archive_current_wal(
    file: &mut File,
    archived_path: &Path,
    committed_offset: u64,
) -> PersistResult<()> {
    let (tmp_path, mut archive) = create_archive_tmp(archived_path)?;
    let result = (|| -> PersistResult<()> {
        file.seek(SeekFrom::Start(0))?;
        let copied = {
            let mut source = (&mut *file).take(committed_offset);
            std::io::copy(&mut source, &mut archive)?
        };
        if copied != committed_offset {
            return Err(PersistError::TruncatedEntry { offset: copied });
        }
        archive.sync_data()?;
        drop(archive);
        publish_archive_tmp(&tmp_path, archived_path)
    })();
    file.seek(SeekFrom::Start(committed_offset))?;
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

fn create_archive_tmp(archived_path: &Path) -> PersistResult<(PathBuf, File)> {
    for attempt in 0..128_u8 {
        let tmp_path =
            archived_path.with_extension(format!("archive.tmp.{}.{}", std::process::id(), attempt));
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .truncate(false)
            .open(&tmp_path)
        {
            Ok(file) => return Ok((tmp_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "wal archive temporary path attempts exhausted",
    )
    .into())
}

fn publish_archive_tmp(tmp_path: &Path, archived_path: &Path) -> PersistResult<()> {
    let already_published = match std::fs::hard_link(tmp_path, archived_path) {
        Ok(()) => false,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => true,
        Err(error) => return Err(PersistError::Io(error)),
    };
    let _ = std::fs::remove_file(tmp_path);
    if already_published {
        return Err(PersistError::WalArchiveExists {
            path: archived_path.to_path_buf(),
        });
    }
    // Parent-dir fsync AFTER the hard_link publish so the new archive directory
    // entry is durable. Per the MANIFEST commit-point invariant the archive's
    // own sync_data (archive_current_wal) precedes its publish, and this dir
    // fsync follows it.
    if let Some(parent) = archived_path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

pub(crate) fn reset_active_wal_file(
    file: &mut File,
    wal_path: &Path,
    snapshot_seq: u64,
) -> PersistResult<()> {
    let (tmp_path, mut replacement) = create_reset_tmp(wal_path)?;
    let mut renamed = false;
    let result = (|| -> PersistResult<()> {
        match replacement.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(PersistError::WriterLockHeld);
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        // Preserve the active WAL's access policy; replacement creation must
        // not silently widen a deliberately restrictive mode such as 0600.
        replacement.set_permissions(file.metadata()?.permissions())?;
        WalFileHeader::new(snapshot_seq).write_to(&mut replacement)?;
        replacement.sync_all()?;
        replacement.seek(SeekFrom::Start(WAL_FILE_HEADER_LEN as u64))?;
        #[cfg(test)]
        fail_reset_at(1)?;

        // Replacing the directory entry is the only destructive operation.
        // Before it, recovery sees the intact old WAL; after it, recovery sees
        // a fully written and synced new header. A crash before the following
        // directory fsync may retain either entry, and both are valid.
        std::fs::rename(&tmp_path, wal_path)?;
        renamed = true;
        #[cfg(test)]
        fail_reset_at(2)?;
        if let Some(parent) = wal_path.parent() {
            sync_dir(parent)?;
        }
        *file = replacement;
        Ok(())
    })();
    if result.is_err() && !renamed {
        let _ = std::fs::remove_file(tmp_path);
    }
    result
}

fn create_reset_tmp(wal_path: &Path) -> PersistResult<(PathBuf, File)> {
    for attempt in 0..128_u8 {
        let mut name = wal_path
            .file_name()
            .expect("active WAL path always has a file name")
            .to_os_string();
        name.push(format!(".reset.tmp.{}.{attempt}", std::process::id()));
        let path = wal_path.with_file_name(name);
        match OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "active WAL reset temporary path attempts exhausted",
    )
    .into())
}

#[cfg(test)]
std::thread_local! {
    static RESET_FAULT_POINT: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn fail_reset_at(point: u8) -> PersistResult<()> {
    RESET_FAULT_POINT.with(|configured| {
        if configured.get() == point {
            configured.set(0);
            Err(std::io::Error::other("injected active WAL reset failure").into())
        } else {
            Ok(())
        }
    })
}

/// In-memory state the [`crate::WalWriter`] adopts after a MANIFEST rotation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RotationCommitState {
    /// New high-water sequence (== the committed snapshot sequence).
    pub last_sequence: u64,
    /// New WAL-header snapshot sequence after the Phase-4 reset.
    pub snapshot_seq: u64,
    /// New committed offset (the post-reset header length).
    pub committed_offset: u64,
}

/// Per-rotation inputs the orchestrator needs from the live writer.
pub(crate) struct RotationInputs<'a> {
    /// The live WAL file handle.
    pub file: &'a mut File,
    /// The active WAL path (used to derive the archive path + parent dir).
    pub wal_path: &'a Path,
    /// Durable offset of the last fully committed WAL entry.
    pub committed_offset: u64,
    /// Writer high-water sequence; must equal the snapshot sequence.
    pub last_sequence: u64,
    /// Snapshot epoch recorded in the current active WAL header.
    pub snapshot_seq: u64,
    /// Whether a committed MANIFEST was present before this rotation.
    pub manifest_present: bool,
    /// Already-archived sequences carried by the live MANIFEST (ascending).
    pub prior_archived_seqs: Vec<u64>,
}

/// Run the crash-safe 4-phase rotate, committing at the MANIFEST rename.
///
/// The MANIFEST write in Phase 3 is the single linearization / commit point for
/// the new epoch. A first-rotation baseline MANIFEST only makes the current
/// epoch authoritative; it does not advance it. Everything before Phase 3 is
/// non-destructive — the previous epoch stays fully recoverable — and
/// everything after it is redundant cleanup. The
/// required ordering for advancing to snapshot epoch `N`
/// (`builder.sequence == last_sequence == N`):
///
/// - **Baseline** (first rotation only): if no MANIFEST exists, validate any
///   non-zero snapshot named by the current WAL header and durably commit that
///   previous epoch before Phase 1. This prevents legacy highest-snapshot
///   recovery from adopting an orphan created by the first rotation.
/// - **Phase 1** (non-destructive): finalize `snapshot.N.snap` then
///   `sync_dir`. The old `snapshot.{N-1}.snap` and the live MANIFEST (still
///   naming `N-1`) are untouched.
/// - **Phase 2** (non-destructive): archive the committed WAL bytes to
///   `wal.{N}.archive` then `sync_dir`. The active `wal.log` is only seeked,
///   never truncated.
/// - **Phase 3** (**COMMIT**): `Manifest::write_atomic` names `live_snapshot_seq
///   = N`; the atomic rename is the instant `N` becomes authoritative.
/// - **Phase 4** (safe-after-commit replacement): write and sync a fresh WAL
///   header seeded with `N`, atomically rename it over `wal.log`, fsync the
///   directory, and only then swap the writer's live handle. Snapshot `N`
///   already captures the replaced `<= N` entries.
///
/// # Why the reset must follow the commit
///
/// Per the MANIFEST commit-point invariant (audit Item 2 / Seam F): if the WAL
/// reset replacement ran *before* the MANIFEST commit and a crash occurred,
/// recovery would read the OLD MANIFEST (epoch `N-1`) but find `wal.log` no
/// longer carrying the `N-1..N` entries — permanent committed-data loss. Hence
/// reset is strictly gated on the Phase-3 rename + dir fsync.
///
/// # Idempotent re-rotate
///
/// Re-invoking after a mid-rotate crash converges: Phase 1/2 re-publish hit
/// `create_new`/`hard_link` `AlreadyExists` (treated as already-done), Phase 3
/// rewrites the MANIFEST to the same epoch, and Phase 4 reset is naturally
/// idempotent.
///
/// # Errors
///
/// Returns [`PersistError::WalRotationSequenceMismatch`] when `builder`'s
/// sequence does not equal `inputs.last_sequence`, plus any I/O / format error
/// from snapshot finalize, archive, MANIFEST commit, or WAL reset. A failure in
/// Phases 1-3 leaves the previous epoch fully recoverable; a failure in Phase 4
/// (after commit) surfaces [`PersistError::WalRotationIncomplete`] but the new
/// epoch is already durable and recovery converges on re-open.
pub(crate) fn rotate_with_manifest(
    inputs: RotationInputs<'_>,
    builder: SnapshotBuilder,
    dir: &Path,
) -> PersistResult<(WalRotationOutcome, RotationCommitState)> {
    let RotationInputs {
        file,
        wal_path,
        committed_offset,
        last_sequence,
        snapshot_seq: prior_snapshot_seq,
        manifest_present,
        prior_archived_seqs,
    } = inputs;

    let snapshot_seq = builder.sequence();
    if snapshot_seq == 0 || last_sequence == 0 {
        return Err(PersistError::WalRotationZeroSequence);
    }
    if snapshot_seq != last_sequence {
        return Err(PersistError::WalRotationSequenceMismatch {
            snapshot_seq,
            last_sequence,
        });
    }

    // A MANIFEST-less directory has no authoritative epoch while Phase 1 is
    // publishing the first new snapshot. Commit a durable baseline first so a
    // crash during Phase 1 cannot make legacy recovery select the orphan. A
    // non-zero WAL header is only a valid baseline when its snapshot exists and
    // verifies; never commit a MANIFEST that would name a missing/corrupt file.
    if !manifest_present {
        if prior_snapshot_seq != 0 {
            let prior_snapshot = snapshot_path(dir, prior_snapshot_seq);
            let mut reader = SnapshotReader::open(&prior_snapshot)?;
            reader.verify_body_hash()?;
            drop(reader);
            // Make the baseline snapshot and its directory entry durable before
            // the baseline MANIFEST makes that epoch authoritative.
            OpenOptions::new()
                .write(true)
                .open(prior_snapshot)?
                .sync_all()?;
            sync_dir(dir)?;
        }
        Manifest {
            live_snapshot_seq: prior_snapshot_seq,
            active_wal_header_seq: prior_snapshot_seq,
            compaction_epoch: 0,
            active_wal: DEFAULT_WAL_FILE_NAME.to_owned(),
            archived_wal_seqs: Vec::new(),
        }
        .write_atomic(dir)?;
    }

    // Phase 1 — publish the snapshot (idempotent: a re-rotate after a crash
    // sees the already-published snapshot.N.snap and treats it as done).
    let snapshot_file = snapshot_path(dir, snapshot_seq);
    match builder.finalize() {
        Ok(_) => {}
        Err(PersistError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    // Durability barrier (audit Item 2 / Seam F): the rotate primitive itself
    // guarantees the published snapshot's bytes are durable BEFORE the Phase-3
    // MANIFEST names it the live epoch — independent of the caller's
    // `SnapshotConfig.fsync`. `SnapshotBuilder::finalize` only `sync_data`s the
    // body when the caller opted into fsync, so without this barrier a crash
    // after the Phase-3 commit could leave the MANIFEST naming a snapshot whose
    // body never reached disk; recovery would then hard-fail on the body hash.
    // Re-opening the published file and `sync_all`ing flushes its inode
    // regardless of which handle wrote it, and is idempotent on the AlreadyExists
    // re-rotate branch. The directory-entry fsync follows the file fsync.
    OpenOptions::new()
        .write(true)
        .open(&snapshot_file)?
        .sync_all()?;
    sync_dir(dir)?;

    // Phase 2 — archive the current WAL (idempotent via WalArchiveExists).
    let archived_path = wal_archive_path(wal_path, last_sequence);
    match archive_current_wal(file, &archived_path, committed_offset) {
        Ok(()) => {}
        Err(PersistError::WalArchiveExists { .. }) => {}
        Err(error) => return Err(error),
    }

    // Phase 3 — COMMIT. The atomic rename is the linearization point.
    let mut archived_wal_seqs = prior_archived_seqs;
    if !archived_wal_seqs.contains(&last_sequence) {
        archived_wal_seqs.push(last_sequence);
    }
    archived_wal_seqs.sort_unstable();
    let manifest = Manifest {
        live_snapshot_seq: snapshot_seq,
        active_wal_header_seq: snapshot_seq,
        compaction_epoch: 0,
        active_wal: DEFAULT_WAL_FILE_NAME.to_owned(),
        archived_wal_seqs,
    };
    manifest.write_atomic(dir)?;

    // Phase 4 — safe-after-commit atomic WAL replacement.
    if reset_active_wal_file(file, wal_path, snapshot_seq).is_err() {
        return Err(PersistError::WalRotationIncomplete {
            archived_path,
            new_path: wal_path.to_path_buf(),
        });
    }

    Ok((
        WalRotationOutcome {
            archived_path,
            new_path: wal_path.to_path_buf(),
            archived_last_sequence: last_sequence,
        },
        RotationCommitState {
            last_sequence: snapshot_seq,
            snapshot_seq,
            committed_offset: WAL_FILE_HEADER_LEN as u64,
        },
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use selene_core::{Change, HlcTimestamp, NodeId, Origin};

    use super::*;
    use crate::{
        RetentionPolicy, SectionCompression, SnapshotBuilder, SnapshotConfig, SyncPolicy,
        WalConfig, WalReader, WalWriter,
    };

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "selene-wal-reset-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        dir
    }

    fn open_locked_wal(path: &Path, snapshot_seq: u64) -> File {
        let mut file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        file.try_lock().unwrap();
        WalFileHeader::new(snapshot_seq)
            .write_to(&mut file)
            .unwrap();
        file.sync_all().unwrap();
        file.seek(SeekFrom::Start(WAL_FILE_HEADER_LEN as u64))
            .unwrap();
        file
    }

    fn handle_snapshot_seq(file: &mut File) -> u64 {
        file.seek(SeekFrom::Start(0)).unwrap();
        let sequence = WalFileHeader::read_from(&mut *file).unwrap().snapshot_seq;
        file.seek(SeekFrom::Start(WAL_FILE_HEADER_LEN as u64))
            .unwrap();
        sequence
    }

    #[test]
    fn active_wal_reset_atomically_replaces_handle_and_keeps_lock() {
        let dir = temp_dir("success");
        let path = dir.join(DEFAULT_WAL_FILE_NAME);
        let mut file = open_locked_wal(&path, 3);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        reset_active_wal_file(&mut file, &path, 9).unwrap();

        assert_eq!(handle_snapshot_seq(&mut file), 9);
        assert_eq!(WalReader::open(&path).unwrap().snapshot_seq(), 9);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(matches!(
            WalWriter::open(&path, WalConfig::default()),
            Err(PersistError::WriterLockHeld)
        ));
        drop(file);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn active_wal_reset_failure_before_rename_preserves_old_wal() {
        let dir = temp_dir("before-rename");
        let path = dir.join(DEFAULT_WAL_FILE_NAME);
        let mut file = open_locked_wal(&path, 3);
        RESET_FAULT_POINT.with(|point| point.set(1));

        let error = reset_active_wal_file(&mut file, &path, 9).unwrap_err();

        assert!(matches!(error, PersistError::Io(_)));
        assert_eq!(handle_snapshot_seq(&mut file), 3);
        assert_eq!(WalReader::open(&path).unwrap().snapshot_seq(), 3);
        assert!(fs::read_dir(&dir).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".reset.tmp.")
        }));
        drop(file);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn active_wal_reset_failure_after_rename_leaves_new_header_valid() {
        let dir = temp_dir("after-rename");
        let path = dir.join(DEFAULT_WAL_FILE_NAME);
        let mut file = open_locked_wal(&path, 3);
        RESET_FAULT_POINT.with(|point| point.set(2));

        let error = reset_active_wal_file(&mut file, &path, 9).unwrap_err();

        assert!(matches!(error, PersistError::Io(_)));
        // The caller still owns the old unlinked handle, but the recovery path
        // names the fully synced replacement header. Rotation treats this as
        // incomplete and requires reopen rather than continuing on the handle.
        assert_eq!(handle_snapshot_seq(&mut file), 3);
        assert_eq!(WalReader::open(&path).unwrap().snapshot_seq(), 9);
        drop(file);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn incomplete_rotation_poisons_all_mutating_writer_apis() {
        let dir = temp_dir("poisoned-writer");
        let path = dir.join(DEFAULT_WAL_FILE_NAME);
        let mut writer = WalWriter::open(
            &path,
            WalConfig {
                sync_policy: SyncPolicy::OnFlushOnly,
                snapshot_seq: 0,
            },
        )
        .unwrap();
        let changes = [Change::NodeDeleted { id: NodeId::new(1) }];
        writer
            .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes)
            .unwrap();
        let builder = SnapshotBuilder::new(SnapshotConfig {
            dir: dir.clone(),
            sequence: 1,
            compression: SectionCompression::None,
            fsync: true,
        });
        RESET_FAULT_POINT.with(|point| point.set(2));

        assert!(matches!(
            writer.rotate_with_manifest(builder),
            Err(PersistError::WalRotationIncomplete { .. })
        ));
        assert!(matches!(
            writer.append(HlcTimestamp::new(2, 0), Origin::Local, None, &changes),
            Err(PersistError::WalWriterPoisoned)
        ));
        assert!(matches!(
            writer.flush(),
            Err(PersistError::WalWriterPoisoned)
        ));
        let retry = SnapshotBuilder::new(SnapshotConfig {
            dir: dir.clone(),
            sequence: 1,
            compression: SectionCompression::None,
            fsync: true,
        });
        assert!(matches!(
            writer.rotate_with_manifest(retry),
            Err(PersistError::WalWriterPoisoned)
        ));
        assert!(matches!(
            writer.prune(&RetentionPolicy::default()),
            Err(PersistError::WalWriterPoisoned)
        ));

        drop(writer);
        fs::remove_dir_all(dir).unwrap();
    }
}
