//! Per-directory serialization for MANIFEST epoch mutations.
//!
//! Cooperating handles and processes on a supported local filesystem use one
//! persistent lock-file inode. A writer's lock order is its lifetime
//! `wal.log` lock, then this epoch lock, then any replacement-WAL temporary
//! lock. The persistence directory, its ancestors, and any symlink aliases must
//! not be renamed, replaced, or retargeted while an operation is live; broader
//! path anchoring is a separate concern from epoch serialization.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::{PersistError, PersistResult};

/// Filename of the persistent lock that serializes MANIFEST epoch operations.
///
/// The file is coordination state, not recovery data. It must never be
/// unlinked or replaced while a process may still hold it, because a new inode
/// would create a second, independent lock domain. The operating system
/// releases the advisory lock when the guard is dropped or its process exits,
/// while the named file remains in place for later operations.
pub const MANIFEST_LOCK_FILE_NAME: &str = "MANIFEST.lock";

/// Exclusive RAII guard for one persistence directory's MANIFEST epoch.
pub(crate) struct ManifestEpochGuard {
    dir: PathBuf,
    _file: File,
}

impl ManifestEpochGuard {
    /// Acquire the directory's stable epoch lock, blocking behind another
    /// cooperating rotation, prune, or direct MANIFEST publication.
    pub(crate) fn acquire(dir: &Path) -> PersistResult<Self> {
        let dir = cwd_independent_directory_path(dir)?;
        let path = dir.join(MANIFEST_LOCK_FILE_NAME);
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                #[cfg(test)]
                run_contention_hook();
                file.lock()?;
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(PersistError::Io(error));
            }
        }
        Ok(Self { dir, _file: file })
    }

    /// CWD-independent absolute directory path protected by this guard.
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }
}

pub(crate) fn cwd_independent_directory_path(path: &Path) -> std::io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

#[cfg(test)]
thread_local! {
    static CONTENTION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_contention_hook(hook: impl FnOnce() + 'static) {
    CONTENTION_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn run_contention_hook() {
    CONTENTION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
#[path = "manifest_lock/tests.rs"]
mod tests;
