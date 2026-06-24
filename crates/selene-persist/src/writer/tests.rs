use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use selene_core::{Change, NodeId, Origin, PropertyMap, Value, db_string};

use super::*;
use crate::{MAX_PRINCIPAL_BYTES, WAL_FILE_HEADER_LEN, WalReader};

fn temp_path(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "selene-persist-{name}-{}-{nanos}.wal",
        std::process::id()
    ))
}

fn changes() -> Vec<Change> {
    vec![Change::NodeCreated {
        id: NodeId::new(1),
        labels: selene_core::LabelSet::single(db_string("writer.node").unwrap()),
        properties: PropertyMap::new(),
    }]
}

fn bytes_change(byte_count: usize) -> Vec<Change> {
    vec![Change::NodeCreated {
        id: NodeId::new(1),
        labels: selene_core::LabelSet::single(db_string("writer.node").unwrap()),
        properties: PropertyMap::from_pairs([(
            db_string("writer.bytes").unwrap(),
            Value::Bytes(Arc::from(vec![0x5a_u8; byte_count])),
        )])
        .unwrap(),
    }]
}

#[test]
fn open_new_file_writes_header() {
    let path = temp_path("open-new");
    {
        let writer = WalWriter::open(&path, WalConfig::default()).unwrap();
        assert_eq!(writer.last_sequence(), 0);
    }
    assert_eq!(
        fs::metadata(&path).unwrap().len(),
        WAL_FILE_HEADER_LEN as u64
    );
    let _ = fs::remove_file(path);
}

#[test]
fn append_assigns_monotonic_sequences() {
    let path = temp_path("seq");
    let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
    assert_eq!(
        writer
            .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
            .unwrap(),
        1
    );
    assert_eq!(
        writer
            .append(HlcTimestamp::new(1, 1), Origin::Local, None, &changes())
            .unwrap(),
        2
    );
    assert_eq!(writer.last_sequence(), 2);
    let _ = fs::remove_file(path);
}

#[test]
fn record_buffer_reuse_does_not_retain_huge_entries() {
    let path = temp_path("record-buffer-cap");
    let mut writer = WalWriter::open_with_compression(
        &path,
        WalConfig {
            sync_policy: SyncPolicy::OnFlushOnly,
            snapshot_seq: 0,
        },
        WalCompression::disabled(),
    )
    .unwrap();
    writer
        .append(
            HlcTimestamp::new(1, 0),
            Origin::Local,
            None,
            &bytes_change(WAL_RECORD_BUFFER_RETAIN_LIMIT + 1024),
        )
        .unwrap();
    assert_eq!(writer.record.capacity(), 0);

    writer
        .append(HlcTimestamp::new(2, 0), Origin::Local, None, &changes())
        .unwrap();
    assert!(writer.record.capacity() <= WAL_RECORD_BUFFER_RETAIN_LIMIT);
    let _ = fs::remove_file(path);
}

#[test]
fn reopen_recovers_last_sequence() {
    let path = temp_path("reopen");
    {
        let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
        writer
            .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
            .unwrap();
    }
    let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
    assert_eq!(writer.last_sequence(), 1);
    assert_eq!(
        writer
            .append(HlcTimestamp::new(2, 0), Origin::Local, None, &changes())
            .unwrap(),
        2
    );
    let _ = fs::remove_file(path);
}

#[test]
fn principal_overflow_does_not_increment_sequence() {
    let path = temp_path("principal");
    let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
    let len_before = fs::metadata(&path).unwrap().len();
    let err = writer
        .append(
            HlcTimestamp::zero(),
            Origin::Local,
            Some(Arc::from(vec![1_u8; MAX_PRINCIPAL_BYTES + 1])),
            &changes(),
        )
        .unwrap_err();
    assert!(matches!(err, PersistError::PrincipalTooLarge { .. }));
    assert_eq!(writer.last_sequence(), 0);
    assert_eq!(fs::metadata(&path).unwrap().len(), len_before);
    let _ = fs::remove_file(path);
}

#[test]
fn partial_tail_is_truncated_on_open() {
    let path = temp_path("tail");
    {
        let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
        writer
            .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
            .unwrap();
        writer.flush().unwrap();
    }
    let valid_len = fs::metadata(&path).unwrap().len();
    {
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&[0, 1, 2]).unwrap();
    }
    let writer = WalWriter::open(&path, WalConfig::default()).unwrap();
    assert_eq!(writer.last_sequence(), 1);
    assert_eq!(fs::metadata(&path).unwrap().len(), valid_len);
    let _ = fs::remove_file(path);
}

#[test]
fn oversized_payload_len_in_tail_truncates_on_open() {
    let path = temp_path("tail-payload-oversize");
    {
        let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
        writer
            .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
            .unwrap();
        writer.flush().unwrap();
    }
    let valid_len = fs::metadata(&path).unwrap().len();
    // Write a 32-byte fixed-prefix header whose payload_len is u32::MAX
    // (> MAX_WAL_ENTRY_BYTES). read_entry_header returns PayloadTooLarge;
    // scan_existing must treat this as torn-tail and truncate back.
    let mut garbage = [0_u8; 32];
    garbage[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
    {
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&garbage).unwrap();
    }
    let writer = WalWriter::open(&path, WalConfig::default()).unwrap();
    assert_eq!(writer.last_sequence(), 1);
    assert_eq!(fs::metadata(&path).unwrap().len(), valid_len);
    let _ = fs::remove_file(path);
}

#[test]
fn oversized_principal_len_in_tail_truncates_on_open() {
    let path = temp_path("tail-principal-oversize");
    {
        let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
        writer
            .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
            .unwrap();
        writer.flush().unwrap();
    }
    let valid_len = fs::metadata(&path).unwrap().len();
    // Fixed-prefix with valid payload_len = 0 but principal_len_p1 large
    // enough that principal_len > MAX_PRINCIPAL_BYTES. The header decoder
    // reports PrincipalTooLarge, and scan_existing must treat that as a
    // torn tail rather than a durable corruption.
    let mut garbage = [0_u8; 32];
    let oversized_p1 = (MAX_PRINCIPAL_BYTES as u16).saturating_add(2);
    garbage[30..32].copy_from_slice(&oversized_p1.to_le_bytes());
    {
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&garbage).unwrap();
    }
    let writer = WalWriter::open(&path, WalConfig::default()).unwrap();
    assert_eq!(writer.last_sequence(), 1);
    assert_eq!(fs::metadata(&path).unwrap().len(), valid_len);
    let _ = fs::remove_file(path);
}

#[test]
fn checksum_corrupt_tail_is_truncated_on_open() {
    let path = temp_path("tail-checksum-corrupt");
    let valid_len = {
        let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
        writer
            .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
            .unwrap();
        let valid_len = writer.committed_offset();
        writer
            .append(HlcTimestamp::new(2, 0), Origin::Local, None, &changes())
            .unwrap();
        writer.flush().unwrap();
        valid_len
    };
    {
        let corrupt_offset = valid_len + crate::entry_header::FIXED_ENTRY_HEADER_BYTES as u64;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.seek(SeekFrom::Start(corrupt_offset)).unwrap();
        let mut byte = [0_u8; 1];
        file.read_exact(&mut byte).unwrap();
        byte[0] ^= 0xFF;
        file.seek(SeekFrom::Start(corrupt_offset)).unwrap();
        file.write_all(&byte).unwrap();
        file.sync_data().unwrap();
    }

    let writer = WalWriter::open(&path, WalConfig::default()).unwrap();
    assert_eq!(writer.last_sequence(), 1);
    assert_eq!(writer.committed_offset(), valid_len);
    assert_eq!(fs::metadata(&path).unwrap().len(), valid_len);
    let _ = fs::remove_file(path);
}

#[test]
fn group_commit_defers_fsync_until_threshold() {
    let path = temp_path("group");
    let mut writer = WalWriter::open(
        &path,
        WalConfig {
            sync_policy: SyncPolicy::EveryN(3),
            snapshot_seq: 0,
        },
    )
    .unwrap();
    assert_eq!(writer.sync_policy.as_every_n(), Some(3));
    writer
        .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
        .unwrap();
    writer
        .append(HlcTimestamp::new(1, 1), Origin::Local, None, &changes())
        .unwrap();
    assert_eq!(writer.entries_since_fsync, 2);
    writer
        .append(HlcTimestamp::new(1, 2), Origin::Local, None, &changes())
        .unwrap();
    assert_eq!(writer.entries_since_fsync, 0);
    let _ = fs::remove_file(path);
}

#[test]
fn wal_config_default_is_every_n_1() {
    assert_eq!(WalConfig::default().sync_policy, SyncPolicy::EveryN(1));
}

#[test]
fn with_fsync_every_n_preserves_legacy_threshold() {
    assert_eq!(
        WalConfig::with_fsync_every_n(7),
        WalConfig {
            sync_policy: SyncPolicy::EveryN(7),
            snapshot_seq: 0,
        }
    );
}

#[test]
fn every_n_zero_normalizes_to_one_on_open() {
    let path = temp_path("normalize");
    let writer = WalWriter::open(
        &path,
        WalConfig {
            sync_policy: SyncPolicy::EveryN(0),
            snapshot_seq: 0,
        },
    )
    .unwrap();
    assert_eq!(writer.sync_policy.as_every_n(), Some(1));
    let _ = fs::remove_file(path);
}

#[test]
fn on_flush_only_accumulates_until_explicit_flush() {
    let path = temp_path("on-flush");
    let mut writer = WalWriter::open(
        &path,
        WalConfig {
            sync_policy: SyncPolicy::OnFlushOnly,
            snapshot_seq: 0,
        },
    )
    .unwrap();
    for tick in 0..3 {
        writer
            .append(HlcTimestamp::new(1, tick), Origin::Local, None, &changes())
            .unwrap();
    }
    assert_eq!(writer.entries_since_fsync, 3);
    writer.flush().unwrap();
    assert_eq!(writer.entries_since_fsync, 0);
    let _ = fs::remove_file(path);
}

#[test]
fn on_flush_only_drop_does_not_fsync() {
    assert!(!SyncPolicy::OnFlushOnly.syncs_on_drop());
    assert!(SyncPolicy::EveryN(1).syncs_on_drop());
}

#[test]
fn replicated_origin_round_trips_node_id_and_source_seq() {
    let path = temp_path("origin");
    {
        let mut writer = WalWriter::open(&path, WalConfig::default()).unwrap();
        writer
            .append(
                HlcTimestamp::new(1, 0),
                Origin::Replicated {
                    source_node_id: NodeId::new(77),
                    source_seq: 9,
                },
                None,
                &changes(),
            )
            .unwrap();
    }
    let reader = WalReader::open(&path).unwrap();
    let entry = reader.iterate(|_| true).unwrap().next().unwrap().unwrap();
    assert_eq!(
        entry.header.origin,
        Origin::Replicated {
            source_node_id: NodeId::new(77),
            source_seq: 9,
        }
    );
    let _ = fs::remove_file(path);
}

#[test]
fn snapshot_seq_seeds_first_appended_sequence() {
    let path = temp_path("snapshot-seq-seed");
    let mut writer = WalWriter::open(
        &path,
        WalConfig {
            sync_policy: SyncPolicy::EveryN(1),
            snapshot_seq: 100,
        },
    )
    .unwrap();
    // First append must follow snapshot_seq + 1 = 101, not start at 1.
    let seq = writer
        .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
        .unwrap();
    assert_eq!(seq, 101);
    assert_eq!(writer.last_sequence(), 101);
    let _ = fs::remove_file(path);
}

#[test]
fn reopen_uses_header_snapshot_seq_not_config() {
    let path = temp_path("reopen-snapshot-seq");
    // Create with snapshot_seq=100
    {
        let mut writer = WalWriter::open(
            &path,
            WalConfig {
                sync_policy: SyncPolicy::EveryN(1),
                snapshot_seq: 100,
            },
        )
        .unwrap();
        assert_eq!(writer.last_sequence(), 100);
        writer
            .append(HlcTimestamp::new(1, 0), Origin::Local, None, &changes())
            .unwrap();
    }
    // Reopen with a stale config.snapshot_seq=0 — header wins, last
    // appended sequence (101) is recovered.
    let writer = WalWriter::open(
        &path,
        WalConfig {
            sync_policy: SyncPolicy::EveryN(1),
            snapshot_seq: 0,
        },
    )
    .unwrap();
    assert_eq!(writer.last_sequence(), 101);
    let _ = fs::remove_file(path);
}

#[test]
fn concurrent_open_admits_exactly_one_writer() {
    // The exclusive OS lock is the single-writer invariant. Race N threads to
    // open the same WAL: exactly one must win (Ok), every other must observe
    // WriterLockHeld — never a second writer corrupting the log. Every winning
    // writer is parked in a shared vec and NOT dropped until all threads have
    // finished their open attempt, so a winner's lock cannot be released and
    // re-acquired mid-race. That makes the count deterministically exactly one.
    use std::sync::{Arc as StdArc, Barrier, Mutex};

    let path = temp_path("concurrent-lock");
    // Ensure the file exists first so every thread races the lock, not create.
    drop(WalWriter::open(&path, WalConfig::default()).unwrap());

    const THREADS: usize = 8;
    let start = StdArc::new(Barrier::new(THREADS));
    // Winners deposit their live writer here; the lock stays held until the
    // vec is dropped after every join.
    let winners: StdArc<Mutex<Vec<WalWriter>>> = StdArc::new(Mutex::new(Vec::new()));
    let path = StdArc::new(path);
    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let start = StdArc::clone(&start);
            let winners = StdArc::clone(&winners);
            let path = StdArc::clone(&path);
            std::thread::spawn(move || {
                start.wait();
                match WalWriter::open(&path, WalConfig::default()) {
                    Ok(writer) => {
                        winners.lock().unwrap().push(writer);
                        true
                    }
                    Err(PersistError::WriterLockHeld) => false,
                    Err(other) => panic!("unexpected open error: {other:?}"),
                }
            })
        })
        .collect();

    let mut wins = 0_usize;
    for handle in handles {
        if handle.join().unwrap() {
            wins += 1;
        }
    }
    assert_eq!(
        wins, 1,
        "exactly one concurrent writer may hold the exclusive lock"
    );
    assert_eq!(winners.lock().unwrap().len(), 1);
    // Drop the single winning writer (releasing the OS lock) before cleanup.
    winners.lock().unwrap().clear();
    let _ = fs::remove_file(path.as_path());
}

#[test]
fn second_writer_open_returns_writer_lock_held() {
    let path = temp_path("lock");
    let _first = WalWriter::open(&path, WalConfig::default()).unwrap();
    let second = WalWriter::open(&path, WalConfig::default());
    assert!(matches!(second, Err(PersistError::WriterLockHeld)));
    drop(_first);
    // After the first writer drops, a fresh open must succeed (the
    // OS releases the file lock when the File handle closes).
    let third = WalWriter::open(&path, WalConfig::default());
    assert!(third.is_ok());
    drop(third);
    let _ = fs::remove_file(path);
}
