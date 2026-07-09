#![allow(missing_docs)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use selene_core::{Change, HlcTimestamp, NodeId, Origin};
use selene_persist::{
    DEFAULT_WAL_FILE_NAME, Manifest, PersistError, SectionCompression, SnapshotBuilder,
    SnapshotConfig, SyncPolicy, WalConfig, WalReader, WalWriter, snapshot_path,
};

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-wal-rotate-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).expect("temp dir is created");
    dir
}

fn changes(id: u64) -> [Change; 1] {
    [Change::NodeDeleted {
        id: NodeId::new(id),
    }]
}

fn append(writer: &mut WalWriter, id: u64) -> u64 {
    writer
        .append(HlcTimestamp::new(id, 0), Origin::Local, None, &changes(id))
        .expect("wal append succeeds")
}

fn sequences(path: &Path) -> Vec<u64> {
    WalReader::open(path)
        .expect("wal opens for read")
        .iterate(|_| true)
        .expect("wal iterates")
        .map(|entry| entry.expect("entry reads").header.sequence)
        .collect()
}

fn meta_builder(dir: &Path, seq: u64, meta: &[u8]) -> SnapshotBuilder {
    let mut builder = SnapshotBuilder::new(SnapshotConfig {
        dir: dir.to_path_buf(),
        sequence: seq,
        compression: SectionCompression::None,
        fsync: true,
    });
    builder
        .add_section(*b"CORE", *b"META", meta.to_vec())
        .expect("section adds");
    builder
}

#[test]
fn wal_rotate_round_trips_new_and_archived_logs() {
    let dir = temp_dir("roundtrip");
    let wal_path = dir.join(DEFAULT_WAL_FILE_NAME);
    let mut writer = WalWriter::open(
        &wal_path,
        WalConfig {
            sync_policy: SyncPolicy::EveryN(1),
            snapshot_seq: 0,
        },
    )
    .expect("wal opens");

    assert_eq!(append(&mut writer, 1), 1);

    let rotation = writer
        .rotate_with_manifest(meta_builder(&dir, writer.last_sequence(), b"snap1"))
        .expect("wal rotates after snapshot");

    assert_eq!(rotation.archived_last_sequence, 1);
    assert_eq!(rotation.archived_path, dir.join("wal.1.archive"));
    assert_eq!(rotation.new_path, wal_path);
    assert!(rotation.archived_path.exists());
    assert!(snapshot_path(&dir, 1).exists());
    assert_eq!(writer.snapshot_seq(), 1);
    assert_eq!(writer.last_sequence(), 1);

    // The MANIFEST is the commit point and names the live epoch + archive set.
    let manifest = Manifest::read(&dir)
        .expect("manifest reads")
        .expect("present");
    assert_eq!(manifest.live_snapshot_seq, 1);
    assert_eq!(manifest.active_wal_header_seq, 1);
    assert_eq!(manifest.active_wal, DEFAULT_WAL_FILE_NAME);
    assert_eq!(manifest.archived_wal_seqs, vec![1]);

    assert_eq!(append(&mut writer, 2), 2);
    writer.flush().expect("rotated wal flushes");

    let archived_reader = WalReader::open(&rotation.archived_path).expect("archive opens");
    assert_eq!(archived_reader.snapshot_seq(), 0);
    assert_eq!(sequences(&rotation.archived_path), vec![1]);

    let new_reader = WalReader::open(&wal_path).expect("new wal opens");
    assert_eq!(new_reader.snapshot_seq(), 1);
    assert_eq!(sequences(&wal_path), vec![2]);

    drop(writer);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn wal_rotate_rejects_builder_sequence_that_does_not_cover_current_wal() {
    let dir = temp_dir("stale-snapshot");
    let wal_path = dir.join(DEFAULT_WAL_FILE_NAME);
    let mut writer = WalWriter::open(&wal_path, WalConfig::default()).expect("wal opens");
    assert_eq!(append(&mut writer, 1), 1);
    assert_eq!(append(&mut writer, 2), 2);
    writer.flush().expect("wal flushes");

    // Builder targets sequence 1 while the writer high-water mark is 2.
    let error = writer
        .rotate_with_manifest(meta_builder(&dir, 1, b"stale"))
        .expect_err("stale snapshot rejects");
    assert!(matches!(
        error,
        PersistError::WalRotationSequenceMismatch {
            snapshot_seq: 1,
            last_sequence: 2
        }
    ));
    // Nothing was published or committed: no archive, no snapshot, no MANIFEST.
    assert!(!dir.join("wal.2.archive").exists());
    assert!(!snapshot_path(&dir, 1).exists());
    assert!(Manifest::read(&dir).expect("manifest reads").is_none());
    assert_eq!(sequences(&wal_path), vec![1, 2]);
    assert_eq!(append(&mut writer, 3), 3);

    drop(writer);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn wal_rotate_with_manifest_is_idempotent_on_pre_published_archive() {
    // A crashed rotation can leave `snapshot.N.snap` and `wal.N.archive`
    // published before the MANIFEST commit. Re-rotating to the same epoch must
    // converge (Phase 1/2 treat AlreadyExists as already-done) rather than
    // hard-fail the way the deleted snapshot-less `rotate` would on a collision.
    let dir = temp_dir("idempotent-rerotate");
    let wal_path = dir.join(DEFAULT_WAL_FILE_NAME);
    let mut writer = WalWriter::open(&wal_path, WalConfig::default()).expect("wal opens");
    assert_eq!(append(&mut writer, 1), 1);
    writer.flush().expect("wal flushes");

    // Pre-publish the archive a crashed Phase 2 would have left behind.
    let archive_path = dir.join("wal.1.archive");
    fs::copy(&wal_path, &archive_path).expect("pre-publish archive");
    assert!(archive_path.exists());

    let rotation = writer
        .rotate_with_manifest(meta_builder(&dir, 1, b"snap1"))
        .expect("rotate converges over pre-published archive");
    assert_eq!(rotation.archived_last_sequence, 1);
    assert_eq!(rotation.archived_path, archive_path);

    let manifest = Manifest::read(&dir)
        .expect("manifest reads")
        .expect("present");
    assert_eq!(manifest.live_snapshot_seq, 1);
    assert_eq!(manifest.archived_wal_seqs, vec![1]);
    assert_eq!(writer.last_sequence(), 1);
    assert_eq!(sequences(&wal_path), Vec::<u64>::new());

    drop(writer);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn rotation_rejects_nondefault_wal_before_flush_or_artifacts() {
    let dir = temp_dir("nondefault-name");
    let wal_path = dir.join("custom.log");
    let mut writer = WalWriter::open(
        &wal_path,
        WalConfig {
            sync_policy: SyncPolicy::OnFlushOnly,
            snapshot_seq: 0,
        },
    )
    .unwrap();
    assert_eq!(append(&mut writer, 1), 1);

    let error = writer
        .rotate_with_manifest(meta_builder(&dir, 1, b"snapshot"))
        .unwrap_err();

    assert!(matches!(
        error,
        PersistError::UnexpectedActiveWal { observed, expected }
            if observed == "custom.log" && expected == DEFAULT_WAL_FILE_NAME
    ));
    assert_eq!(writer.entries_since_fsync(), 1);
    assert!(Manifest::read(&dir).unwrap().is_none());
    assert!(!snapshot_path(&dir, 1).exists());
    assert!(!dir.join("wal.1.archive").exists());
    drop(writer);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn rotation_rejects_nondefault_manifest_wal_before_flush_or_artifacts() {
    let dir = temp_dir("nondefault-manifest-wal");
    let wal_path = dir.join(DEFAULT_WAL_FILE_NAME);
    let invalid_manifest = Manifest {
        live_snapshot_seq: 0,
        active_wal_header_seq: 0,
        compaction_epoch: 0,
        active_wal: "custom.log".to_owned(),
        archived_wal_seqs: Vec::new(),
    };
    invalid_manifest
        .write_atomic(&dir)
        .expect("invalid manifest fixture writes");
    let mut writer = WalWriter::open(
        &wal_path,
        WalConfig {
            sync_policy: SyncPolicy::OnFlushOnly,
            snapshot_seq: 0,
        },
    )
    .expect("wal opens");
    assert_eq!(append(&mut writer, 1), 1);

    let error = writer
        .rotate_with_manifest(meta_builder(&dir, 1, b"snapshot"))
        .expect_err("non-default manifest active WAL rejects");

    assert!(matches!(
        error,
        PersistError::UnexpectedActiveWal { observed, expected }
            if observed == "custom.log" && expected == DEFAULT_WAL_FILE_NAME
    ));
    assert_eq!(writer.entries_since_fsync(), 1);
    assert_eq!(
        Manifest::read(&dir).expect("manifest reads"),
        Some(invalid_manifest)
    );
    assert!(!snapshot_path(&dir, 1).exists());
    assert!(!dir.join("wal.1.archive").exists());
    drop(writer);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn rotation_rejects_zero_sequence_before_artifacts() {
    let dir = temp_dir("zero-sequence");
    let wal_path = dir.join(DEFAULT_WAL_FILE_NAME);
    let mut writer = WalWriter::open(&wal_path, WalConfig::default()).unwrap();

    let error = writer
        .rotate_with_manifest(meta_builder(&dir, 0, b"zero"))
        .unwrap_err();

    assert!(matches!(error, PersistError::WalRotationZeroSequence));
    assert!(Manifest::read(&dir).unwrap().is_none());
    assert!(!snapshot_path(&dir, 0).exists());
    assert!(!dir.join("wal.0.archive").exists());
    drop(writer);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn rotation_rejects_snapshot_directory_mismatch_before_flush() {
    let dir = temp_dir("directory-mismatch");
    let other = temp_dir("directory-mismatch-other");
    let wal_path = dir.join(DEFAULT_WAL_FILE_NAME);
    let mut writer = WalWriter::open(
        &wal_path,
        WalConfig {
            sync_policy: SyncPolicy::OnFlushOnly,
            snapshot_seq: 0,
        },
    )
    .unwrap();
    assert_eq!(append(&mut writer, 1), 1);

    let error = writer
        .rotate_with_manifest(meta_builder(&other, 1, b"wrong-dir"))
        .unwrap_err();

    assert!(matches!(
        error,
        PersistError::WalRotationDirectoryMismatch {
            snapshot_dir,
            wal_dir,
        } if snapshot_dir == other && wal_dir == dir
    ));
    assert_eq!(writer.entries_since_fsync(), 1);
    assert!(Manifest::read(&dir).unwrap().is_none());
    assert!(!snapshot_path(&other, 1).exists());
    drop(writer);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(other);
}
