#![allow(missing_docs)]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use selene_core::{Change, HlcTimestamp, NodeId, Origin};
use selene_persist::{SyncPolicy, WAL_FILE_HEADER_LEN, WalConfig, WalWriter};

fn temp_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "selene-wal-state-{name}-{}-{nanos}.log",
        std::process::id()
    ))
}

#[test]
fn wal_state_accessors_reflect_committed_append_and_flush() {
    let path = temp_path("accessors");
    let mut writer = WalWriter::open(
        &path,
        WalConfig {
            sync_policy: SyncPolicy::OnFlushOnly,
            snapshot_seq: 41,
        },
    )
    .expect("wal opens");

    assert_eq!(writer.snapshot_seq(), 41);
    assert_eq!(writer.last_sequence(), 41);
    assert_eq!(writer.committed_offset(), WAL_FILE_HEADER_LEN as u64);
    assert_eq!(writer.entries_since_fsync(), 0);

    let changes = [Change::NodeDeleted { id: NodeId::new(7) }];
    let sequence = writer
        .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes)
        .expect("wal append succeeds");

    assert_eq!(sequence, 42);
    assert_eq!(writer.last_sequence(), 42);
    assert!(writer.committed_offset() > WAL_FILE_HEADER_LEN as u64);
    assert_eq!(writer.entries_since_fsync(), 1);

    writer.flush().expect("wal flush succeeds");
    assert_eq!(writer.entries_since_fsync(), 0);

    drop(writer);
    let _ = fs::remove_file(path);
}
