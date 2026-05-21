#![allow(missing_docs)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use selene_core::{Change, HlcTimestamp, NodeId, Origin};
use selene_persist::{
    DEFAULT_WAL_FILE_NAME, PersistError, SectionCompression, SnapshotBuilder, SnapshotConfig,
    SyncPolicy, WalConfig, WalReader, WalWriter, snapshot_path,
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
    let snapshot = SnapshotBuilder::new(SnapshotConfig {
        dir: dir.clone(),
        sequence: writer.last_sequence(),
        compression: SectionCompression::None,
        fsync: true,
    })
    .finalize()
    .expect("snapshot finalizes");

    let rotation = writer
        .rotate(snapshot.snapshot_seq)
        .expect("wal rotates after snapshot");

    assert_eq!(rotation.archived_last_sequence, 1);
    assert_eq!(rotation.archived_path, dir.join("wal.1.archive"));
    assert_eq!(rotation.new_path, wal_path);
    assert!(rotation.archived_path.exists());
    assert!(snapshot_path(&dir, snapshot.snapshot_seq).exists());
    assert_eq!(writer.snapshot_seq(), 1);
    assert_eq!(writer.last_sequence(), 1);

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
fn wal_rotate_existing_archive_is_rejected_without_data_loss() {
    let dir = temp_dir("no-clobber");
    let wal_path = dir.join(DEFAULT_WAL_FILE_NAME);
    let mut writer = WalWriter::open(&wal_path, WalConfig::default()).expect("wal opens");
    assert_eq!(append(&mut writer, 1), 1);
    writer.flush().expect("wal flushes");

    let archive_path = dir.join("wal.1.archive");
    fs::write(&archive_path, b"existing archive").expect("archive fixture writes");

    let error = writer.rotate(1).expect_err("archive collision rejects");
    assert!(matches!(
        error,
        PersistError::WalArchiveExists { path } if path == archive_path
    ));
    assert_eq!(fs::read(&archive_path).unwrap(), b"existing archive");
    assert_eq!(sequences(&wal_path), vec![1]);
    assert_eq!(append(&mut writer, 2), 2);

    drop(writer);
    let _ = fs::remove_dir_all(dir);
}
