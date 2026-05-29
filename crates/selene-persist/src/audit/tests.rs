//! Unit tests for the append-only audit log.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::*;

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-audit-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).expect("temp dir created");
    dir
}

fn log_path(dir: &Path) -> PathBuf {
    dir.join(DEFAULT_AUDIT_FILE_NAME)
}

fn record(at: u64, kind: u16, payload: &[u8]) -> AuditRecord {
    AuditRecord {
        recorded_at_unix_nanos: at,
        kind,
        payload: payload.to_vec(),
    }
}

#[test]
fn open_creates_file_with_header() {
    let dir = temp_dir("create");
    let path = log_path(&dir);
    let _log = AuditLog::open(&path).unwrap();
    assert!(path.exists());
    // A fresh log has the 8-byte header and no records.
    assert_eq!(AuditLog::read_all(&path).unwrap(), Vec::new());
    let bytes = fs::read(&path).unwrap();
    assert_eq!(&bytes[0..4], &AUDIT_MAGIC);
    assert_eq!(bytes.len(), 8);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn append_then_read_all_round_trips() {
    let dir = temp_dir("round-trip");
    let path = log_path(&dir);
    let mut log = AuditLog::open(&path).unwrap();
    let r = record(1_000, AUDIT_KIND_PACK_LIFECYCLE, b"activate pack alpha");
    log.append(&r).unwrap();
    assert_eq!(AuditLog::read_all(&path).unwrap(), vec![r]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn multiple_records_preserve_oldest_first_order() {
    let dir = temp_dir("order");
    let path = log_path(&dir);
    let mut log = AuditLog::open(&path).unwrap();
    let recs: Vec<AuditRecord> = (0..5)
        .map(|i| {
            record(
                100 + i,
                AUDIT_KIND_PACK_LIFECYCLE,
                format!("ev{i}").as_bytes(),
            )
        })
        .collect();
    for r in &recs {
        log.append(r).unwrap();
    }
    assert_eq!(AuditLog::read_all(&path).unwrap(), recs);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn empty_payload_round_trips() {
    let dir = temp_dir("empty-payload");
    let path = log_path(&dir);
    let mut log = AuditLog::open(&path).unwrap();
    let r = record(7, 9, b"");
    log.append(&r).unwrap();
    assert_eq!(AuditLog::read_all(&path).unwrap(), vec![r]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn reopen_appends_not_overwrites() {
    let dir = temp_dir("reopen");
    let path = log_path(&dir);
    {
        let mut log = AuditLog::open(&path).unwrap();
        log.append(&record(1, 1, b"first")).unwrap();
    }
    {
        let mut log = AuditLog::open(&path).unwrap();
        log.append(&record(2, 1, b"second")).unwrap();
    }
    let all = AuditLog::read_all(&path).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].payload, b"first");
    assert_eq!(all[1].payload, b"second");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn open_truncates_torn_trailing_record() {
    let dir = temp_dir("torn-tail");
    let path = log_path(&dir);
    {
        let mut log = AuditLog::open(&path).unwrap();
        log.append(&record(1, 1, b"good")).unwrap();
    }
    // Simulate a crash mid-append: a partial record header tacked onto the end.
    {
        use std::io::Write;
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(&[0xAB; 10]).unwrap(); // shorter than a full 20-byte header
    }
    // Re-open discards the torn tail and truncates back to the good record.
    let log = AuditLog::open(&path).unwrap();
    drop(log);
    let all = AuditLog::read_all(&path).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].payload, b"good");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn open_truncates_checksum_corrupted_tail() {
    let dir = temp_dir("corrupt-tail");
    let path = log_path(&dir);
    {
        let mut log = AuditLog::open(&path).unwrap();
        log.append(&record(1, 1, b"keep")).unwrap();
        log.append(&record(2, 1, b"corruptme")).unwrap();
    }
    // Flip the last byte (inside the second record's payload).
    let mut bytes = fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    fs::write(&path, &bytes).unwrap();

    let _ = AuditLog::open(&path).unwrap(); // truncates the corrupt record
    let all = AuditLog::read_all(&path).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].payload, b"keep");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn append_after_torn_tail_recovery_is_contiguous() {
    let dir = temp_dir("append-after-torn");
    let path = log_path(&dir);
    {
        let mut log = AuditLog::open(&path).unwrap();
        log.append(&record(1, 1, b"good")).unwrap();
    }
    {
        use std::io::Write;
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(&[0xAB; 10]).unwrap();
    }
    // Re-open (truncates torn tail) then append: the new record lands right
    // after the good one, with no garbage between.
    {
        let mut log = AuditLog::open(&path).unwrap();
        log.append(&record(2, 1, b"after")).unwrap();
    }
    let all = AuditLog::read_all(&path).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].payload, b"good");
    assert_eq!(all[1].payload, b"after");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn payload_too_large_is_rejected() {
    let dir = temp_dir("too-large");
    let path = log_path(&dir);
    let mut log = AuditLog::open(&path).unwrap();
    let big = vec![0_u8; MAX_AUDIT_PAYLOAD_BYTES + 1];
    let err = log.append(&record(1, 1, &big)).unwrap_err();
    assert!(matches!(err, PersistError::PayloadTooLarge { .. }));
    // The rejected append wrote nothing.
    assert_eq!(AuditLog::read_all(&path).unwrap(), Vec::new());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn read_all_absent_file_is_empty() {
    let dir = temp_dir("absent");
    assert_eq!(
        AuditLog::read_all(&dir.join("does-not-exist.log")).unwrap(),
        Vec::new()
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn corrupt_file_header_is_rejected() {
    let dir = temp_dir("bad-header");
    let path = log_path(&dir);
    AuditLog::open(&path).unwrap();

    // Wrong magic.
    let mut bytes = fs::read(&path).unwrap();
    bytes[0..4].copy_from_slice(b"NOPE");
    fs::write(&path, &bytes).unwrap();
    assert!(matches!(
        AuditLog::read_all(&path),
        Err(PersistError::MagicMismatch { .. })
    ));

    // Unsupported version.
    let mut bytes = fs::read(&path).unwrap();
    bytes[0..4].copy_from_slice(&AUDIT_MAGIC);
    bytes[4..6].copy_from_slice(&9_u16.to_le_bytes());
    fs::write(&path, &bytes).unwrap();
    assert!(matches!(
        AuditLog::read_all(&path),
        Err(PersistError::UnsupportedVersion { major: 9, .. })
    ));

    // Nonzero reserved.
    let mut bytes = fs::read(&path).unwrap();
    bytes[4..6].copy_from_slice(&AUDIT_FORMAT_VERSION.to_le_bytes());
    bytes[6] = 1;
    fs::write(&path, &bytes).unwrap();
    assert!(matches!(
        AuditLog::read_all(&path),
        Err(PersistError::ReservedBytesNonZero { offset: 6 })
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn prune_keep_n_events_drops_oldest() {
    let dir = temp_dir("prune-count");
    let path = log_path(&dir);
    let mut log = AuditLog::open(&path).unwrap();
    for i in 0..6 {
        log.append(&record(100 + i, 1, format!("e{i}").as_bytes()))
            .unwrap();
    }
    let policy = AuditRetentionPolicy {
        keep_n_events: Some(2),
        max_age: None,
    };
    let outcome = log.prune(&policy, 1_000).unwrap();
    assert_eq!(outcome.retained, 2);
    assert_eq!(outcome.removed, 4);
    assert!(outcome.bytes_reclaimed > 0);

    let all = AuditLog::read_all(&path).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].payload, b"e4"); // newest two retained, oldest-first
    assert_eq!(all[1].payload, b"e5");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn prune_max_age_drops_old_records() {
    let dir = temp_dir("prune-age");
    let path = log_path(&dir);
    let mut log = AuditLog::open(&path).unwrap();
    // Three records at increasing wall-clock stamps (nanoseconds).
    log.append(&record(1_000, 1, b"old")).unwrap();
    log.append(&record(5_000, 1, b"mid")).unwrap();
    log.append(&record(9_000, 1, b"new")).unwrap();

    // now = 10_000ns, max_age = 6_000ns → cutoff 4_000; only "old" (1_000) drops.
    let policy = AuditRetentionPolicy {
        keep_n_events: None,
        max_age: Some(Duration::from_nanos(6_000)),
    };
    let outcome = log.prune(&policy, 10_000).unwrap();
    assert_eq!(outcome.removed, 1);
    assert_eq!(outcome.retained, 2);
    let all = AuditLog::read_all(&path).unwrap();
    assert_eq!(
        all.iter().map(|r| r.payload.clone()).collect::<Vec<_>>(),
        vec![b"mid".to_vec(), b"new".to_vec()]
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn prune_conjunctive_count_and_age() {
    let dir = temp_dir("prune-both");
    let path = log_path(&dir);
    let mut log = AuditLog::open(&path).unwrap();
    for i in 0..5 {
        // stamps 1000, 2000, 3000, 4000, 5000
        log.append(&record(1_000 * (i + 1), 1, format!("e{i}").as_bytes()))
            .unwrap();
    }
    // now=5500, max_age=3000 → cutoff 2500 keeps {3000,4000,5000} (e2,e3,e4);
    // keep_n_events=2 then keeps the newest 2 of those → {e3,e4}.
    let policy = AuditRetentionPolicy {
        keep_n_events: Some(2),
        max_age: Some(Duration::from_nanos(3_000)),
    };
    let outcome = log.prune(&policy, 5_500).unwrap();
    assert_eq!(outcome.retained, 2);
    assert_eq!(outcome.removed, 3);
    let all = AuditLog::read_all(&path).unwrap();
    assert_eq!(
        all.iter().map(|r| r.payload.clone()).collect::<Vec<_>>(),
        vec![b"e3".to_vec(), b"e4".to_vec()]
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn prune_noop_when_nothing_exceeds_policy() {
    let dir = temp_dir("prune-noop");
    let path = log_path(&dir);
    let mut log = AuditLog::open(&path).unwrap();
    log.append(&record(1, 1, b"a")).unwrap();
    log.append(&record(2, 1, b"b")).unwrap();

    let policy = AuditRetentionPolicy {
        keep_n_events: Some(10),
        max_age: None,
    };
    let outcome = log.prune(&policy, 1_000).unwrap();
    assert_eq!(outcome.retained, 2);
    assert_eq!(outcome.removed, 0);
    assert_eq!(outcome.bytes_reclaimed, 0);
    assert_eq!(AuditLog::read_all(&path).unwrap().len(), 2);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn prune_then_append_continues_contiguously() {
    let dir = temp_dir("prune-then-append");
    let path = log_path(&dir);
    let mut log = AuditLog::open(&path).unwrap();
    for i in 0..4 {
        log.append(&record(100 + i, 1, format!("e{i}").as_bytes()))
            .unwrap();
    }
    log.prune(
        &AuditRetentionPolicy {
            keep_n_events: Some(1),
            max_age: None,
        },
        1_000,
    )
    .unwrap();
    // The same handle keeps working after the rewrite.
    log.append(&record(200, 1, b"post")).unwrap();
    let all = AuditLog::read_all(&path).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].payload, b"e3"); // last survivor
    assert_eq!(all[1].payload, b"post");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn prune_leaves_no_tmp_file() {
    let dir = temp_dir("prune-no-tmp");
    let path = log_path(&dir);
    let mut log = AuditLog::open(&path).unwrap();
    for i in 0..3 {
        log.append(&record(i, 1, b"x")).unwrap();
    }
    log.prune(
        &AuditRetentionPolicy {
            keep_n_events: Some(1),
            max_age: None,
        },
        1_000,
    )
    .unwrap();
    assert!(!dir.join("audit.log.tmp").exists());
    assert!(path.exists());
    let _ = fs::remove_dir_all(dir);
}
