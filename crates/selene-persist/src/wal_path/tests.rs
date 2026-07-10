use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use selene_core::{Change, HlcTimestamp, NodeId, Origin};

use super::*;
use crate::{WalConfig, WalWriter};

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-wal-path-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn append_one(writer: &mut WalWriter) {
    writer
        .append(
            HlcTimestamp::new(1, 0),
            Origin::Local,
            None,
            &[Change::NodeDeleted { id: NodeId::new(1) }],
        )
        .unwrap();
}

#[cfg(unix)]
#[test]
fn parent_alias_is_anchored_before_the_final_path_opens() {
    use std::os::unix::fs::symlink;

    let root = temp_dir("parent-retarget");
    let first_dir = root.join("first");
    let second_dir = root.join("second");
    let alias = root.join("live");
    std::fs::create_dir(&first_dir).unwrap();
    std::fs::create_dir(&second_dir).unwrap();
    symlink(&first_dir, &alias).unwrap();

    let hook_alias = alias.clone();
    let hook_second = second_dir.clone();
    set_after_parent_anchor_hook(move || {
        std::fs::remove_file(&hook_alias).unwrap();
        symlink(&hook_second, &hook_alias).unwrap();
    });
    let mut first = WalWriter::open(&alias.join("wal.log"), WalConfig::default()).unwrap();
    let mut second = WalWriter::open(&alias.join("wal.log"), WalConfig::default()).unwrap();

    assert_eq!(first.path(), first_dir.join("wal.log"));
    assert_eq!(second.path(), second_dir.join("wal.log"));
    assert!(matches!(
        WalWriter::open(&first_dir.join("wal.log"), WalConfig::default()),
        Err(PersistError::WriterLockHeld)
    ));
    assert!(matches!(
        WalWriter::open(&second_dir.join("wal.log"), WalConfig::default()),
        Err(PersistError::WriterLockHeld)
    ));
    append_one(&mut first);
    append_one(&mut second);
    first.flush().unwrap();
    second.flush().unwrap();
    drop(first);
    drop(second);
    let first = WalWriter::open(&first_dir.join("wal.log"), WalConfig::default()).unwrap();
    let second = WalWriter::open(&second_dir.join("wal.log"), WalConfig::default()).unwrap();
    assert_eq!(first.last_sequence(), 1);
    assert_eq!(second.last_sequence(), 1);
    drop(first);
    drop(second);

    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn final_symlink_is_rejected_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let dir = temp_dir("final-symlink");
    let backing = dir.join("backing.log");
    drop(WalWriter::open(&backing, WalConfig::default()).unwrap());
    let before = std::fs::read(&backing).unwrap();
    let active = dir.join("wal.log");
    symlink(&backing, &active).unwrap();

    let error = match WalWriter::open(&active, WalConfig::default()) {
        Ok(_) => panic!("final symlink must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        PersistError::WalPathNotRegular { path } if path == active
    ));
    assert_eq!(std::fs::read(&backing).unwrap(), before);
    drop(WalWriter::open(&backing, WalConfig::default()).unwrap());

    std::fs::remove_file(&active).unwrap();
    symlink(dir.join("missing.log"), &active).unwrap();
    assert!(matches!(
        WalWriter::open(&active, WalConfig::default()),
        Err(PersistError::WalPathNotRegular { path }) if path == active
    ));

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn final_directory_is_rejected_without_blocking_on_open() {
    let dir = temp_dir("final-directory");
    let active = dir.join("wal.log");
    std::fs::create_dir(&active).unwrap();

    assert!(matches!(
        WalWriter::open(&active, WalConfig::default()),
        Err(PersistError::WalPathNotRegular { path }) if path == active
    ));

    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn hard_link_alias_still_contends_on_the_open_inode() {
    let dir = temp_dir("hard-link");
    let active = dir.join("wal.log");
    let alias = dir.join("wal.alias");
    let writer = WalWriter::open(&active, WalConfig::default()).unwrap();
    std::fs::hard_link(&active, &alias).unwrap();

    assert!(matches!(
        WalWriter::open(&alias, WalConfig::default()),
        Err(PersistError::WriterLockHeld)
    ));

    drop(writer);
    std::fs::remove_dir_all(dir).unwrap();
}
