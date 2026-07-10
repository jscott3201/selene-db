//! Anchored, regular-file-only active WAL opening.
//!
//! The portable `std` checks assume a cooperating persistence directory whose
//! resolved ancestors and final entry are not replaced during open. They reject
//! stable symlinks/non-files but are not hostile rename-resistant `openat` or
//! no-follow capability semantics.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::file_header::WalFileHeader;
use crate::manifest::sync_dir;
use crate::manifest_lock::canonical_directory_path;
use crate::{PersistError, PersistResult};

const OPEN_RACE_RETRIES: usize = 8;
const INIT_TEMP_RETRIES: usize = 32;
static WAL_INIT_NONCE: AtomicU64 = AtomicU64::new(0);

/// Open and exclusively lock a WAL through its canonical parent directory.
///
/// Parent aliases are resolved once before the final component is inspected.
/// Existing final entries must be regular files. An absent WAL is initialized
/// and fsynced under a unique sibling name, then hard-linked into the final path
/// with fail-on-existing semantics. Readers can therefore observe only absence
/// or a complete header, while the returned handle retains the inode lock that
/// was acquired before publication.
pub(crate) fn open_locked_wal(
    path: &Path,
    initial_snapshot_seq: u64,
) -> PersistResult<(File, PathBuf)> {
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "WAL path must name a file",
        )
    })?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let dir = canonical_directory_path(parent)?;
    #[cfg(test)]
    run_after_parent_anchor_hook();
    let path = dir.join(file_name);

    for _ in 0..OPEN_RACE_RETRIES {
        let file = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                require_regular_metadata(&path, &metadata)?;
                OpenOptions::new().read(true).write(true).open(&path)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match create_initialized_wal(&dir, &path, file_name, initial_snapshot_seq)? {
                    Some(file) => return Ok((file, path)),
                    None => continue,
                }
            }
            Err(error) => return Err(error.into()),
        };
        if !file.metadata()?.is_file() {
            return Err(PersistError::WalPathNotRegular { path });
        }
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(PersistError::WriterLockHeld);
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        require_regular_wal(&path)?;
        return Ok((file, path));
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "active WAL path changed repeatedly while opening",
    )
    .into())
}

fn create_initialized_wal(
    dir: &Path,
    final_path: &Path,
    file_name: &std::ffi::OsStr,
    snapshot_seq: u64,
) -> PersistResult<Option<File>> {
    let (mut file, temp_path) = create_init_temp(dir, file_name)?;
    if let Err(error) = file.try_lock() {
        cleanup_init_temp(file, &temp_path);
        return match error {
            std::fs::TryLockError::WouldBlock => Err(PersistError::WriterLockHeld),
            std::fs::TryLockError::Error(error) => Err(error.into()),
        };
    }
    if let Err(error) = (|| -> PersistResult<()> {
        WalFileHeader::new(snapshot_seq).write_to(&mut file)?;
        file.sync_all()?;
        Ok(())
    })() {
        cleanup_init_temp(file, &temp_path);
        return Err(error);
    }

    #[cfg(test)]
    run_before_wal_publish_hook();
    match std::fs::hard_link(&temp_path, final_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            cleanup_init_temp(file, &temp_path);
            return Ok(None);
        }
        Err(error) => {
            cleanup_init_temp(file, &temp_path);
            return Err(error.into());
        }
    }
    if let Err(error) = std::fs::remove_file(&temp_path) {
        drop(file);
        let _ = std::fs::remove_file(&temp_path);
        return Err(error.into());
    }
    sync_dir(dir)?;
    Ok(Some(file))
}

fn create_init_temp(dir: &Path, file_name: &std::ffi::OsStr) -> PersistResult<(File, PathBuf)> {
    for _ in 0..INIT_TEMP_RETRIES {
        let nonce = WAL_INIT_NONCE.fetch_add(1, Ordering::Relaxed);
        let mut temp_name = file_name.to_os_string();
        temp_name.push(format!(".init.{}.{nonce}.tmp", std::process::id()));
        let temp_path = dir.join(temp_name);
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((file, temp_path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a unique WAL initialization temporary",
    )
    .into())
}

fn cleanup_init_temp(file: File, temp_path: &Path) {
    drop(file);
    if let Err(error) = std::fs::remove_file(temp_path) {
        tracing::warn!(path = %temp_path.display(), %error, "could not remove WAL initialization temporary");
    }
}

/// Reject a present WAL path unless its directory entry is a regular file.
pub(crate) fn require_regular_wal_or_absent(path: &Path) -> PersistResult<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => require_regular_metadata(path, &metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn require_regular_wal(path: &Path) -> PersistResult<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    require_regular_metadata(path, &metadata)
}

fn require_regular_metadata(path: &Path, metadata: &std::fs::Metadata) -> PersistResult<()> {
    if metadata.file_type().is_file() {
        Ok(())
    } else {
        Err(PersistError::WalPathNotRegular {
            path: path.to_path_buf(),
        })
    }
}

#[cfg(test)]
thread_local! {
    static AFTER_PARENT_ANCHOR_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
    static AFTER_ROTATION_PREFLIGHT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
    static BEFORE_WAL_PUBLISH_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_after_parent_anchor_hook(hook: impl FnOnce() + 'static) {
    AFTER_PARENT_ANCHOR_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
pub(crate) fn set_after_rotation_preflight_hook(hook: impl FnOnce() + 'static) {
    AFTER_ROTATION_PREFLIGHT_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
pub(crate) fn set_before_wal_publish_hook(hook: impl FnOnce() + 'static) {
    BEFORE_WAL_PUBLISH_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn run_after_parent_anchor_hook() {
    AFTER_PARENT_ANCHOR_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn run_before_wal_publish_hook() {
    BEFORE_WAL_PUBLISH_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
pub(crate) fn run_after_rotation_preflight_hook() {
    AFTER_ROTATION_PREFLIGHT_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
#[path = "wal_path/tests.rs"]
mod tests;
