//! WAL rotation helpers, including the crash-safe multi-phase rotate
//! orchestrator whose MANIFEST commit is the rotation linearization point.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::file_header::{WAL_FILE_HEADER_LEN, WalFileHeader};
use crate::manifest::{Manifest, sync_dir};
use crate::snapshot_path::snapshot_path;
use crate::snapshot_writer::SnapshotBuilder;
use crate::{DEFAULT_WAL_FILE_NAME, PersistError, PersistResult};

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

pub(crate) fn reset_active_wal_file(file: &mut File, snapshot_seq: u64) -> PersistResult<()> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    WalFileHeader::new(snapshot_seq).write_to(&mut *file)?;
    file.sync_data()?;
    file.seek(SeekFrom::Start(WAL_FILE_HEADER_LEN as u64))?;
    Ok(())
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
    /// Already-archived sequences carried by the live MANIFEST (ascending).
    pub prior_archived_seqs: Vec<u64>,
}

/// Run the crash-safe 4-phase rotate, committing at the MANIFEST rename.
///
/// The MANIFEST write (Phase 3) is the single linearization / commit point.
/// Everything before it is provably non-destructive — the previous epoch stays
/// fully recoverable — and everything after it is redundant cleanup. The
/// required ordering for advancing to snapshot epoch `N`
/// (`builder.sequence == last_sequence == N`):
///
/// - **Phase 1** (non-destructive): finalize `snapshot.N.snap` then
///   `sync_dir`. The old `snapshot.{N-1}.snap` and the live MANIFEST (still
///   naming `N-1`) are untouched.
/// - **Phase 2** (non-destructive): archive the committed WAL bytes to
///   `wal.{N}.archive` then `sync_dir`. The active `wal.log` is only seeked,
///   never truncated.
/// - **Phase 3** (**COMMIT**): `Manifest::write_atomic` names `live_snapshot_seq
///   = N`; the atomic rename is the instant `N` becomes authoritative.
/// - **Phase 4** (safe-after-commit destructive): reset `wal.log` to a fresh
///   header seeded with `N`. Safe because snapshot `N` already captures state
///   through the rotate boundary; the erased `<= N` entries are redundant.
///
/// # Why the reset must follow the commit
///
/// Per the MANIFEST commit-point invariant (audit Item 2 / Seam F): if the WAL
/// reset (`set_len(0)`) ran *before* the MANIFEST commit and a crash occurred,
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
        prior_archived_seqs,
    } = inputs;

    let snapshot_seq = builder.sequence();
    if snapshot_seq != last_sequence {
        return Err(PersistError::WalRotationSequenceMismatch {
            snapshot_seq,
            last_sequence,
        });
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

    // Phase 4 — safe-after-commit destructive WAL reset.
    if reset_active_wal_file(file, snapshot_seq).is_err() {
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
