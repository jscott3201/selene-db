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
    let r = record(1_000, AUDIT_KIND_RESERVED_0, b"reserved event alpha");
    log.append(&r).unwrap();
    assert_eq!(AuditLog::read_all(&path).unwrap(), vec![r]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn decode_all_truncates_garbage_tail() {
    // decode_all reproduces read_all's torn-tail semantics: a trailing partial
    // record (here a short record header) is silently dropped, returning only
    // the durable records, never erroring or panicking.
    let dir = temp_dir("decode-torn");
    let path = log_path(&dir);
    {
        let mut log = AuditLog::open(&path).unwrap();
        log.append(&record(10, 0, b"first")).unwrap();
        log.append(&record(20, AUDIT_KIND_RESERVED_0, b"second"))
            .unwrap();
    }
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend_from_slice(&[0xAB_u8; AUDIT_RECORD_HEADER_LEN - 4]); // torn header
    let decoded = AuditLog::decode_all(&bytes).unwrap();
    assert_eq!(decoded, AuditLog::read_all(&path).unwrap());
    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].payload, b"first");
    assert_eq!(decoded[1].payload, b"second");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn decode_all_empty_and_header_only_are_empty() {
    assert_eq!(AuditLog::decode_all(&[]).unwrap(), Vec::new());
    let mut header = Vec::new();
    header.extend_from_slice(&AUDIT_MAGIC);
    header.extend_from_slice(&AUDIT_FORMAT_VERSION.to_le_bytes());
    header.extend_from_slice(&0_u16.to_le_bytes());
    assert_eq!(AuditLog::decode_all(&header).unwrap(), Vec::new());
}

#[test]
fn decode_all_rejects_corrupt_header() {
    assert!(matches!(
        AuditLog::decode_all(b"NOPExxxx"),
        Err(PersistError::MagicMismatch { .. })
    ));
    // A full-length record header of garbage is refused, not read as a torn
    // tail. Under v1 the over-cap payload_len alone made this "the tail" and
    // decode returned Ok(empty). A torn append cannot produce it: a partial
    // header is short (caught by length) and a complete one carries a valid
    // checksum, so garbage of exactly header length is damage.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&AUDIT_MAGIC);
    bytes.extend_from_slice(&AUDIT_FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&[0xFF_u8; AUDIT_RECORD_HEADER_LEN]);
    assert!(
        matches!(
            AuditLog::decode_all(&bytes),
            Err(PersistError::AuditMidLogCorruption { offset: 8, .. })
        ),
        "a garbage full-length header is corruption, not a tail"
    );
}

/// The zero fill a short extending write leaves is still a tail. This is the
/// one case where a failed header checksum may be repaired, and it is why the
/// classifier probes rather than refusing every unauthenticated header.
#[test]
fn zero_filled_tail_is_still_a_tail() {
    let dir = temp_dir("zero-tail");
    let path = log_path(&dir);
    let mut log = AuditLog::open(&path).unwrap();
    let kept = record(1, AUDIT_KIND_RESERVED_0, b"kept");
    log.append(&kept).unwrap();
    drop(log);

    let durable = fs::metadata(&path).unwrap().len();
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend_from_slice(&[0_u8; 64]);
    fs::write(&path, &bytes).unwrap();

    let _log = AuditLog::open(&path).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().len(),
        durable,
        "the zero fill is discarded and the record survives"
    );
    assert_eq!(AuditLog::read_all(&path).unwrap(), vec![kept]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn multiple_records_preserve_oldest_first_order() {
    let dir = temp_dir("order");
    let path = log_path(&dir);
    let mut log = AuditLog::open(&path).unwrap();
    let recs: Vec<AuditRecord> = (0..5)
        .map(|i| record(100 + i, AUDIT_KIND_RESERVED_0, format!("ev{i}").as_bytes()))
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
        f.write_all(&[0xAB; 10]).unwrap(); // shorter than a full 24-byte header
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
fn read_one_record_near_cap_payload_len_is_treated_as_torn_tail() {
    // PERSIST-25 symmetry: read_one_record now uses saturating_add for
    // payload_end (mirroring the WAL reader). A trailing record header whose
    // payload_len sits just under the cap but runs past EOF must be treated as a
    // torn tail (Ok(None) → truncated), never an arithmetic overflow or panic.
    let dir = temp_dir("near-cap-torn");
    let path = log_path(&dir);
    {
        let mut log = AuditLog::open(&path).unwrap();
        log.append(&record(1, 1, b"keep")).unwrap();
    }
    // Append a torn record header claiming a near-cap payload_len with no payload
    // bytes following it. recorded_at(8) + kind(2) + reserved(2) + payload_len(4)
    // + checksum(4) = 20-byte header.
    {
        use std::io::Write;
        let mut header = [0_u8; 20];
        // recorded_at_unix_nanos = 0 (bytes 0..8 already zero)
        // kind = 1
        header[8..10].copy_from_slice(&1_u16.to_le_bytes());
        // payload_len just under the cap (well past EOF).
        let near_cap = (MAX_AUDIT_PAYLOAD_BYTES as u32) - 1;
        header[12..16].copy_from_slice(&near_cap.to_le_bytes());
        // checksum left zero.
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(&header).unwrap();
    }
    // read_all stops at the torn record without panicking or erroring.
    let all = AuditLog::read_all(&path).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].payload, b"keep");
    // Re-open truncates the torn header back to the durable tail.
    let _log = AuditLog::open(&path).unwrap();
    assert_eq!(AuditLog::read_all(&path).unwrap().len(), 1);
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
        Err(PersistError::UnsupportedVersion {
            artifact: PersistArtifact::AuditLog,
            major: 9,
            ..
        })
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

/// The #1110 defect, in the issue's own terms: 100 records, flip one bit in
/// record 10's payload. v1 deleted records 10-100, fsynced the shortened file,
/// and returned `Ok` — on every recovery, because `SharedGraph::recover`
/// reopens the audit log unconditionally.
#[test]
fn interior_payload_corruption_refuses_and_preserves_the_log() {
    let dir = temp_dir("interior-payload");
    let path = log_path(&dir);
    // Zero-padded so every record encodes to the same length, which is what
    // lets the test compute record 10's offset instead of threading offsets out
    // of the writer.
    let records: Vec<AuditRecord> = (0..100)
        .map(|index| {
            record(
                1_000 + index,
                AUDIT_KIND_RESERVED_0,
                format!("event-{index:03}").as_bytes(),
            )
        })
        .collect();
    {
        let mut log = AuditLog::open(&path).unwrap();
        for entry in &records {
            log.append(entry).unwrap();
        }
    }

    // Record 10's payload. Every record here encodes to the same length, so the
    // offset is computable without threading offsets out of the writer.
    let record_len = AUDIT_RECORD_HEADER_LEN + records[10].payload.len();
    let payload_at = AUDIT_FILE_HEADER_LEN + 10 * record_len + AUDIT_RECORD_HEADER_LEN;
    let mut bytes = fs::read(&path).unwrap();
    bytes[payload_at] ^= 0b0000_0001;
    fs::write(&path, &bytes).unwrap();
    let corrupted = fs::read(&path).unwrap();

    let expected_offset = (AUDIT_FILE_HEADER_LEN + 10 * record_len) as u64;
    let error = AuditLog::open(&path).expect_err("interior damage must refuse");
    let PersistError::AuditMidLogCorruption {
        offset,
        trailing_bytes,
        source,
    } = &error
    else {
        panic!("expected a refusal, got {error:?}");
    };
    assert_eq!(*offset, expected_offset);
    assert_eq!(*trailing_bytes, corrupted.len() as u64 - expected_offset);
    assert!(
        matches!(**source, PersistError::AuditRecordChecksumMismatch { .. }),
        "the refusal must carry the underlying cause, got {source:?}"
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        corrupted,
        "a refusal must leave the file byte-identical: records 11-99 were \
         acknowledged and must survive for offline recovery"
    );

    // And the readers must not quietly hand back the prefix either.
    assert!(matches!(
        AuditLog::read_all(&path),
        Err(PersistError::AuditMidLogCorruption { .. })
    ));
    assert!(matches!(
        AuditLog::decode_all(&corrupted),
        Err(PersistError::AuditMidLogCorruption { .. })
    ));
    let _ = fs::remove_dir_all(dir);
}

/// The field that made the v1 scan unrecoverable: `payload_len` decides where a
/// record ends, so a flip in it used to resynchronize the scan onto a bogus
/// boundary. The header checksum now catches it before the value is used.
#[test]
fn interior_payload_len_corruption_refuses() {
    let dir = temp_dir("interior-len");
    let path = log_path(&dir);
    {
        let mut log = AuditLog::open(&path).unwrap();
        for index in 0..5 {
            log.append(&record(index, AUDIT_KIND_RESERVED_0, b"payload"))
                .unwrap();
        }
    }

    let record_len = AUDIT_RECORD_HEADER_LEN + b"payload".len();
    let record_start = AUDIT_FILE_HEADER_LEN + record_len;
    let mut bytes = fs::read(&path).unwrap();
    // payload_len lives at record-header offset 16 under v2.
    bytes[record_start + 16] ^= 0b0000_0001;
    fs::write(&path, &bytes).unwrap();
    let corrupted = fs::read(&path).unwrap();

    let error = AuditLog::open(&path).expect_err("a flipped length must refuse");
    let PersistError::AuditMidLogCorruption { offset, source, .. } = &error else {
        panic!("expected a refusal, got {error:?}");
    };
    assert_eq!(*offset, record_start as u64);
    assert!(
        matches!(**source, PersistError::AuditHeaderChecksumMismatch { .. }),
        "the length must fail integrity before it is used as an offset, got {source:?}"
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        corrupted,
        "a refusal must not truncate"
    );
    let _ = fs::remove_dir_all(dir);
}

/// A recovery that refuses must keep refusing. v1's truncation was idempotent
/// in the worst way: the second open saw a shorter, self-consistent file and
/// reported success, so the loss became invisible one restart later.
#[test]
fn a_refused_open_stays_refused_across_reopens() {
    let dir = temp_dir("refuse-idempotent");
    let path = log_path(&dir);
    {
        let mut log = AuditLog::open(&path).unwrap();
        for index in 0..4 {
            log.append(&record(index, AUDIT_KIND_RESERVED_0, b"abcd"))
                .unwrap();
        }
    }
    let record_len = AUDIT_RECORD_HEADER_LEN + 4;
    let mut bytes = fs::read(&path).unwrap();
    bytes[AUDIT_FILE_HEADER_LEN + record_len + AUDIT_RECORD_HEADER_LEN] ^= 0xFF;
    fs::write(&path, &bytes).unwrap();
    let corrupted = fs::read(&path).unwrap();

    for attempt in 0..3 {
        assert!(
            matches!(
                AuditLog::open(&path),
                Err(PersistError::AuditMidLogCorruption { .. })
            ),
            "attempt {attempt} must refuse"
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            corrupted,
            "attempt {attempt} must not truncate"
        );
    }
    let _ = fs::remove_dir_all(dir);
}

/// The final record is the one an interrupted append can damage, so it stays
/// repairable. Without this the fix would brick every crash recovery.
#[test]
fn corrupt_final_record_is_still_repaired() {
    let dir = temp_dir("final-record");
    let path = log_path(&dir);
    let kept = record(1, AUDIT_KIND_RESERVED_0, b"kept");
    {
        let mut log = AuditLog::open(&path).unwrap();
        log.append(&kept).unwrap();
        log.append(&record(2, AUDIT_KIND_RESERVED_0, b"torn"))
            .unwrap();
    }
    let durable = (AUDIT_FILE_HEADER_LEN + AUDIT_RECORD_HEADER_LEN + kept.payload.len()) as u64;

    let mut bytes = fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0b0000_0001;
    fs::write(&path, &bytes).unwrap();

    let _log =
        AuditLog::open(&path).expect("the last record has nothing after it, so it is a tail");
    assert_eq!(fs::metadata(&path).unwrap().len(), durable);
    assert_eq!(AuditLog::read_all(&path).unwrap(), vec![kept]);
    let _ = fs::remove_dir_all(dir);
}

/// A v1 log is rejected by version, and the message names the audit log rather
/// than the WAL — the artifact discrimination this format bump makes reachable.
#[test]
fn a_v1_log_is_rejected_naming_the_audit_artifact() {
    let dir = temp_dir("v1-reject");
    let path = log_path(&dir);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&AUDIT_MAGIC);
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    fs::write(&path, &bytes).unwrap();

    let error = AuditLog::open(&path).expect_err("a v1 log must be rejected");
    assert!(
        matches!(
            error,
            PersistError::UnsupportedVersion {
                artifact: PersistArtifact::AuditLog,
                major: 1,
                ..
            }
        ),
        "expected an audit-log version rejection, got {error:?}"
    );
    assert_eq!(
        error.to_string(),
        "audit log version unsupported: 1.0",
        "the message must name the artifact an operator has to recreate"
    );
    let _ = fs::remove_dir_all(dir);
}
