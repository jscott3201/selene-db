//! WAL rotation helpers.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::file_header::{WAL_FILE_HEADER_LEN, WalFileHeader};
use crate::{PersistError, PersistResult};

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

pub(crate) fn wal_archive_path(path: &Path, last_sequence: u64) -> PathBuf {
    let archive_name = format!("wal.{last_sequence}.archive");
    path.parent().map_or_else(
        || PathBuf::from(&archive_name),
        |parent| parent.join(&archive_name),
    )
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
    std::fs::hard_link(tmp_path, archived_path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            PersistError::WalArchiveExists {
                path: archived_path.to_path_buf(),
            }
        } else {
            PersistError::Io(error)
        }
    })?;
    let _ = std::fs::remove_file(tmp_path);
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
