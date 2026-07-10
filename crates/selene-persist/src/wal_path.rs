//! Anchored, regular-file-only active WAL opening.
//!
//! The portable `std` checks assume a cooperating persistence directory whose
//! resolved ancestors and final entry are not replaced during open. They reject
//! stable symlinks/non-files but are not hostile rename-resistant `openat` or
//! no-follow capability semantics.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::manifest_lock::canonical_directory_path;
use crate::{PersistError, PersistResult};

const OPEN_RACE_RETRIES: usize = 8;

/// Open and exclusively lock a WAL through its canonical parent directory.
///
/// Parent aliases are resolved once before the final component is inspected.
/// Existing final entries must be regular files; absent entries are created
/// with `create_new` so an intervening symlink cannot be followed on that path.
pub(crate) fn open_locked_wal(path: &Path) -> PersistResult<(File, PathBuf)> {
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
                match OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&path)
                {
                    Ok(file) => file,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error.into()),
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
fn run_after_parent_anchor_hook() {
    AFTER_PARENT_ANCHOR_HOOK.with(|slot| {
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
