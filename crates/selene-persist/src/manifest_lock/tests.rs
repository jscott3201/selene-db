use std::sync::mpsc::{RecvTimeoutError, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

use super::*;
use crate::{DEFAULT_WAL_FILE_NAME, Manifest};

const CHILD_DIR_ENV: &str = "SELENE_MANIFEST_LOCK_CHILD_DIR";

fn temp_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-manifest-lock-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).unwrap();
    dir
}

fn wait_for_path(path: &Path, phase: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(Instant::now() < deadline, "timed out waiting for {phase}");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn same_directory_guards_serialize_and_lock_file_persists() {
    let dir = temp_dir("same-dir");
    let first = ManifestEpochGuard::acquire(&dir).unwrap();
    let (contended_tx, contended_rx) = sync_channel(0);
    let (acquired_tx, acquired_rx) = sync_channel(0);
    let worker_dir = dir.clone();
    let worker = thread::spawn(move || {
        set_contention_hook(move || contended_tx.send(()).unwrap());
        let _second = ManifestEpochGuard::acquire(&worker_dir).unwrap();
        acquired_tx.send(()).unwrap();
    });

    contended_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("second guard reaches real lock contention");
    assert_eq!(
        acquired_rx.recv_timeout(Duration::from_millis(50)),
        Err(RecvTimeoutError::Timeout)
    );
    drop(first);
    acquired_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("second guard acquires after release");
    worker.join().unwrap();

    assert!(dir.join(MANIFEST_LOCK_FILE_NAME).is_file());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn different_directories_do_not_contend() {
    let first_dir = temp_dir("first-dir");
    let second_dir = temp_dir("second-dir");
    let first = ManifestEpochGuard::acquire(&first_dir).unwrap();

    let (acquired_tx, acquired_rx) = sync_channel(0);
    let worker_dir = second_dir.clone();
    let worker = thread::spawn(move || {
        let _second = ManifestEpochGuard::acquire(&worker_dir).unwrap();
        acquired_tx.send(()).unwrap();
    });
    acquired_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("independent directory lock acquires immediately");
    worker.join().unwrap();

    drop(first);
    std::fs::remove_dir_all(first_dir).unwrap();
    std::fs::remove_dir_all(second_dir).unwrap();
}

#[test]
fn separate_process_contends_on_the_same_lock_file() {
    let dir = temp_dir("separate-process");
    let first = ManifestEpochGuard::acquire(&dir).unwrap();
    let contended = dir.join("child-contended");
    let acquired = dir.join("child-acquired");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "manifest_lock::tests::manifest_lock_child_helper",
            "--ignored",
            "--nocapture",
        ])
        .env(CHILD_DIR_ENV, &dir)
        .spawn()
        .unwrap();

    wait_for_path(&contended, "child-process lock contention");
    assert!(child.try_wait().unwrap().is_none());
    drop(first);
    assert!(child.wait().unwrap().success());
    assert!(acquired.is_file());

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "helper invoked by separate_process_contends_on_the_same_lock_file"]
fn manifest_lock_child_helper() {
    let Some(dir) = std::env::var_os(CHILD_DIR_ENV).map(PathBuf::from) else {
        return;
    };
    let contended = dir.join("child-contended");
    let acquired = dir.join("child-acquired");
    set_contention_hook(move || std::fs::write(contended, []).unwrap());
    let _guard = ManifestEpochGuard::acquire(&dir).unwrap();
    std::fs::write(acquired, []).unwrap();
}

#[test]
fn direct_manifest_publication_uses_the_epoch_lock() {
    let dir = temp_dir("manifest-write");
    let first = ManifestEpochGuard::acquire(&dir).unwrap();
    let (contended_tx, contended_rx) = sync_channel(0);
    let manifest = Manifest {
        live_snapshot_seq: 7,
        active_wal_header_seq: 7,
        compaction_epoch: 0,
        active_wal: DEFAULT_WAL_FILE_NAME.to_owned(),
        archived_wal_seqs: vec![7],
    };
    let worker_dir = dir.clone();
    let expected = manifest.clone();
    let worker = thread::spawn(move || {
        set_contention_hook(move || contended_tx.send(()).unwrap());
        manifest.write_atomic(&worker_dir).unwrap();
    });

    contended_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("MANIFEST writer reaches the held epoch lock");
    assert!(Manifest::read(&dir).unwrap().is_none());
    drop(first);
    worker.join().unwrap();
    assert_eq!(Manifest::read(&dir).unwrap(), Some(expected));

    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn guard_keeps_publication_on_the_resolved_directory_after_alias_retarget() {
    use std::os::unix::fs::symlink;

    let root = temp_dir("alias-retarget").canonicalize().unwrap();
    let first_dir = root.join("first");
    let second_dir = root.join("second");
    let alias = root.join("live");
    std::fs::create_dir(&first_dir).unwrap();
    std::fs::create_dir(&second_dir).unwrap();
    symlink(&first_dir, &alias).unwrap();
    let mut guard = ManifestEpochGuard::acquire(&alias).unwrap();
    assert_eq!(guard.dir(), first_dir);

    std::fs::remove_file(&alias).unwrap();
    symlink(&second_dir, &alias).unwrap();
    let manifest = Manifest {
        live_snapshot_seq: 9,
        active_wal_header_seq: 9,
        compaction_epoch: 0,
        active_wal: DEFAULT_WAL_FILE_NAME.to_owned(),
        archived_wal_seqs: vec![9],
    };
    manifest.write_atomic_locked(&mut guard).unwrap();

    assert_eq!(Manifest::read(&first_dir).unwrap(), Some(manifest));
    assert!(Manifest::read(&second_dir).unwrap().is_none());
    drop(guard);
    std::fs::remove_dir_all(root).unwrap();
}
