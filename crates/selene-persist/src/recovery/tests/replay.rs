//! Recovery WAL-replay tests (mid-stream hard-fail, principal survival,
//! per-commit batch override, empty-entry skip, per-source dedupe,
//! compaction-epoch + active-wal contract). Split from the parent test
//! module to keep each file under the 700-LOC cap.

// `super::*` brings in the parent test module's helpers (temp_dir, change,
// changes, registry, write_snapshot, write_wal, RecordingProvider, Event). The
// crate types below are re-imported explicitly because `use` imports are not
// re-exported through a glob.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use selene_core::{Change, HlcTimestamp, NodeId, Origin};

use super::*;
use crate::{
    DEFAULT_WAL_FILE_NAME, FLAG_CHECKPOINT_WATERMARK, PersistError, ProviderRegistry,
    RecoveryProvider, RecoveryResult, WalEntryHeader, WalReader, recover,
};

/// Hand-assemble a WAL file (header + the supplied entries) so a test can
/// corrupt a specific non-final entry's bytes. Each entry is `Origin::Local`,
/// no principal, carrying `NodeDeleted { id }`. Returns the path and the byte
/// offset of each entry's payload start (parallel to `ids`).
fn write_wal_raw(dir: &Path, snapshot_seq: u64, ids: &[u64]) -> (PathBuf, Vec<usize>) {
    use crate::entry_header::encode_entry_header;
    use crate::file_header::WalFileHeader;
    use crate::payload::encode_changes;

    let path = dir.join(DEFAULT_WAL_FILE_NAME);
    let mut bytes = Vec::new();
    WalFileHeader::new(snapshot_seq)
        .write_to(&mut bytes)
        .unwrap();
    let mut payload_offsets = Vec::with_capacity(ids.len());
    for (index, id) in ids.iter().enumerate() {
        let payload = encode_changes(&[change(*id)]).unwrap();
        let header = WalEntryHeader::new(
            payload.bytes.len(),
            payload.checksum_lo,
            snapshot_seq + index as u64 + 1,
            HlcTimestamp::new(index as u64 + 1, 0),
            Origin::Local,
            payload.flags,
            None,
        )
        .unwrap();
        bytes.extend_from_slice(&encode_entry_header(&header).unwrap());
        payload_offsets.push(bytes.len());
        bytes.extend_from_slice(&payload.bytes);
    }
    fs::write(&path, &bytes).unwrap();
    (path, payload_offsets)
}

/// The same interior-corruption refusal, but reached through snapshot-plus-WAL
/// replay rather than the WAL-only path.
///
/// The two paths differ: with a snapshot the replay floor filters frames by
/// sequence before they are decoded, so the guard that hard-fails on a corrupt
/// interior frame is not obviously the same guard. A snapshot must not let a
/// damaged frame past, and must not let the frames after it disappear.
#[test]
fn recover_corrupt_midstream_entry_hard_fails_with_a_snapshot_present() {
    let dir = temp_dir("snapshot-midstream-checksum");
    write_snapshot(&dir, 100, &[(*b"CORE", *b"META", b"meta".to_vec())]);
    // Entries land at 101, 102, 103 — all above the snapshot's replay floor.
    let (path, payload_offsets) = write_wal_raw(&dir, 100, &[1, 2, 3]);
    let mut bytes = fs::read(&path).unwrap();
    bytes[payload_offsets[1]] ^= 0xFF;
    fs::write(&path, &bytes).unwrap();
    let corrupted = fs::read(&path).unwrap();

    let core = RecordingProvider::new(*b"CORE");
    let err = recover(&dir, &registry(std::slice::from_ref(&core))).unwrap_err();
    assert!(
        matches!(err, PersistError::ChecksumMismatch { sequence: 102 }),
        "expected ChecksumMismatch on entry 102, got {err:?}"
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        corrupted,
        "a refusal must not rewrite the WAL"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_corrupt_midstream_entry_hard_fails() {
    // A torn TAIL silently truncates (tested elsewhere); a corrupt NON-FINAL
    // entry must HARD-FAIL so a regression to silent-break can never lose
    // committed data. Cover both mid-stream guards.

    // (1) ChecksumMismatch on the middle entry (flip a payload byte). Replay
    //     decodes entry 1 (ok), then `into_entry()` on entry 2 verifies the
    //     checksum and the mismatch propagates (only TruncatedEntry breaks).
    {
        let dir = temp_dir("midstream-checksum");
        let (path, payload_offsets) = write_wal_raw(&dir, 0, &[1, 2, 3]);
        let mut bytes = fs::read(&path).unwrap();
        // Corrupt the SECOND entry's payload (non-final).
        bytes[payload_offsets[1]] ^= 0xFF;
        fs::write(&path, &bytes).unwrap();
        let core = RecordingProvider::new(*b"CORE");
        let err = recover(&dir, &registry(std::slice::from_ref(&core))).unwrap_err();
        assert!(
            matches!(err, PersistError::ChecksumMismatch { sequence: 2 }),
            "expected ChecksumMismatch on entry 2, got {err:?}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    // (2) NonMonotonicSequence on a non-final entry hard-fails: entry 3
    //     repeats sequence 2, followed by a real entry 4 so the bad one is
    //     not the tail. The iterator surfaces NonMonotonicSequence and replay
    //     returns it (it only swallows TruncatedEntry).
    {
        let dir = temp_dir("midstream-nonmono");
        build_nonmonotonic_wal(&dir);
        let core = RecordingProvider::new(*b"CORE");
        let err = recover(&dir, &registry(std::slice::from_ref(&core))).unwrap_err();
        assert!(
            matches!(
                err,
                PersistError::NonMonotonicSequence {
                    previous: 2,
                    current: 2
                }
            ),
            "expected NonMonotonicSequence, got {err:?}"
        );
        let _ = fs::remove_dir_all(dir);
    }
}

/// Build a WAL whose third entry repeats the second entry's sequence (2),
/// so iteration reports `NonMonotonicSequence` on a non-final entry.
fn build_nonmonotonic_wal(dir: &Path) -> PathBuf {
    use std::io::Write;

    use crate::entry_header::encode_entry_header;
    use crate::file_header::WalFileHeader;
    use crate::payload::encode_changes;

    let path = dir.join(DEFAULT_WAL_FILE_NAME);
    let mut file = fs::File::create(&path).unwrap();
    WalFileHeader::new(0).write_to(&mut file).unwrap();
    // Sequences 1, 2, then 2 again (the collision on the third, non-final
    // entry), followed by a real fourth entry so the bad one is not the tail.
    for seq in [1_u64, 2, 2, 4] {
        let payload = encode_changes(&[change(seq)]).unwrap();
        let header = WalEntryHeader::new(
            payload.bytes.len(),
            payload.checksum_lo,
            seq,
            HlcTimestamp::new(seq, 0),
            Origin::Local,
            payload.flags,
            None,
        )
        .unwrap();
        file.write_all(&encode_entry_header(&header).unwrap())
            .unwrap();
        file.write_all(&payload.bytes).unwrap();
    }
    path
}

#[test]
fn recover_replays_entries_carrying_principal() {
    // The D12 principal lives in the WAL entry header; pin that it survives a
    // full recover() pass, including at the MAX_PRINCIPAL_BYTES cap.
    use std::io::Write;

    use crate::MAX_PRINCIPAL_BYTES;
    use crate::entry_header::encode_entry_header;
    use crate::file_header::WalFileHeader;
    use crate::payload::encode_changes;

    let dir = temp_dir("principal-replay");
    let path = dir.join(DEFAULT_WAL_FILE_NAME);
    let principals: [Arc<[u8]>; 2] = [
        Arc::from(b"alice".as_slice()),
        Arc::from(vec![0x5A_u8; MAX_PRINCIPAL_BYTES]),
    ];
    {
        let mut file = fs::File::create(&path).unwrap();
        WalFileHeader::new(0).write_to(&mut file).unwrap();
        for (index, principal) in principals.iter().enumerate() {
            let payload = encode_changes(&[change(index as u64 + 1)]).unwrap();
            let header = WalEntryHeader::new(
                payload.bytes.len(),
                payload.checksum_lo,
                index as u64 + 1,
                HlcTimestamp::new(index as u64 + 1, 0),
                Origin::Local,
                payload.flags,
                Some(principal.clone()),
            )
            .unwrap();
            file.write_all(&encode_entry_header(&header).unwrap())
                .unwrap();
            file.write_all(&payload.bytes).unwrap();
        }
    }

    // A header-capturing provider asserts each replayed entry's principal.
    let reader = WalReader::open(&path).unwrap();
    let observed: Vec<Option<Arc<[u8]>>> = reader
        .iterate(|_| true)
        .unwrap()
        .map(|view| view.unwrap().header.principal.clone())
        .collect();
    assert_eq!(observed.len(), 2);
    assert_eq!(observed[0].as_deref(), Some(b"alice".as_slice()));
    assert_eq!(observed[1].as_ref().unwrap().len(), MAX_PRINCIPAL_BYTES);

    // And a full recover() pass applies both committed entries.
    let core = RecordingProvider::new(*b"CORE");
    let outcome = recover(&dir, &registry(std::slice::from_ref(&core))).unwrap();
    assert_eq!(outcome.last_wal_seq, 2);
    assert_eq!(outcome.wal_changes_applied, 2);
    assert_eq!(
        core.events(),
        vec![
            Event::Change(Box::new(change(1))),
            Event::Change(Box::new(change(2)))
        ]
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_delivers_per_commit_batch_to_on_changes_override() {
    // The commit-level on_changes override seam is never overridden by the
    // production providers in these tests; exercise it directly. Build a WAL
    // whose entries carry change batches of sizes [1, 3, 2] and assert a
    // batching provider observes exactly those batch boundaries.
    use std::io::Write;
    use std::sync::Mutex;

    use crate::entry_header::encode_entry_header;
    use crate::file_header::WalFileHeader;
    use crate::payload::encode_changes;

    struct BatchProvider {
        tag: [u8; 4],
        batches: Mutex<Vec<usize>>,
    }
    impl RecoveryProvider for BatchProvider {
        fn provider_tag(&self) -> [u8; 4] {
            self.tag
        }
        fn read_section(&self, _sub: [u8; 4], _bytes: &[u8]) -> RecoveryResult<()> {
            Ok(())
        }
        fn on_changes(&self, changes: &[Change]) -> RecoveryResult<()> {
            self.batches.lock().unwrap().push(changes.len());
            Ok(())
        }
    }

    let dir = temp_dir("batch-override");
    let path = dir.join(DEFAULT_WAL_FILE_NAME);
    let batch_ids: [&[u64]; 3] = [&[1], &[2, 3, 4], &[5, 6]];
    {
        let mut file = fs::File::create(&path).unwrap();
        WalFileHeader::new(0).write_to(&mut file).unwrap();
        for (index, ids) in batch_ids.iter().enumerate() {
            let batch: Vec<Change> = ids.iter().copied().map(change).collect();
            let payload = encode_changes(&batch).unwrap();
            let header = WalEntryHeader::new(
                payload.bytes.len(),
                payload.checksum_lo,
                index as u64 + 1,
                HlcTimestamp::new(index as u64 + 1, 0),
                Origin::Local,
                payload.flags,
                None,
            )
            .unwrap();
            file.write_all(&encode_entry_header(&header).unwrap())
                .unwrap();
            file.write_all(&payload.bytes).unwrap();
        }
    }

    let provider = Arc::new(BatchProvider {
        tag: *b"BTCH",
        batches: Mutex::new(Vec::new()),
    });
    let mut registry = ProviderRegistry::new();
    let dynp: Arc<dyn RecoveryProvider> = provider.clone();
    registry.register(dynp).unwrap();
    let outcome = recover(&dir, &registry).unwrap();
    assert_eq!(outcome.wal_changes_applied, 6);
    assert_eq!(*provider.batches.lock().unwrap(), vec![1, 3, 2]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_distinguishes_empty_commit_from_checkpoint_watermark() {
    // An ordinary empty commit advances logical generation; a flagged
    // checkpoint watermark advances only the physical sequence.
    use std::io::Write;

    use crate::entry_header::encode_entry_header;
    use crate::file_header::WalFileHeader;
    use crate::payload::encode_changes;

    let dir = temp_dir("empty-entry");
    let path = dir.join(DEFAULT_WAL_FILE_NAME);
    {
        let mut file = fs::File::create(&path).unwrap();
        WalFileHeader::new(0).write_to(&mut file).unwrap();
        for (seq, batch, watermark) in [
            (1_u64, vec![change(1)], false),
            (2, Vec::new(), false),
            (3, Vec::new(), true),
            (4, vec![change(4)], false),
        ] {
            let payload = encode_changes(&batch).unwrap();
            let header = WalEntryHeader::new(
                payload.bytes.len(),
                payload.checksum_lo,
                seq,
                HlcTimestamp::new(seq, 0),
                Origin::Local,
                payload.flags
                    | if watermark {
                        FLAG_CHECKPOINT_WATERMARK
                    } else {
                        0
                    },
                None,
            )
            .unwrap();
            file.write_all(&encode_entry_header(&header).unwrap())
                .unwrap();
            file.write_all(&payload.bytes).unwrap();
        }
    }
    let core = RecordingProvider::new(*b"CORE");
    let outcome = recover(&dir, &registry(std::slice::from_ref(&core))).unwrap();
    assert_eq!(outcome.last_wal_seq, 4);
    assert_eq!(outcome.wal_commit_entries_applied, 3);
    assert_eq!(outcome.wal_changes_applied, 2);
    assert_eq!(
        core.events(),
        vec![
            Event::Change(Box::new(change(1))),
            Event::Change(Box::new(change(4)))
        ]
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_replicated_dedupe_is_per_source_node() {
    // Dedupe is keyed by (source_node_id, source_seq); a regression keying by
    // source_seq alone would drop the second source's same-seq entry. Two
    // distinct source nodes both carry source_seq 5 — both must apply.
    let dir = temp_dir("dedupe-per-source");
    let source_a = Origin::Replicated {
        source_node_id: NodeId::new(10),
        source_seq: 5,
    };
    let source_b = Origin::Replicated {
        source_node_id: NodeId::new(20),
        source_seq: 5,
    };
    write_wal(
        &dir,
        0,
        &[
            (source_a, changes(&[1])),
            (source_b, changes(&[2])),
            // A genuine repeat of source_a/seq 5 — this one is deduped.
            (source_a, changes(&[3])),
        ],
    );
    let core = RecordingProvider::new(*b"CORE");
    let outcome = recover(&dir, &registry(std::slice::from_ref(&core))).unwrap();
    assert_eq!(outcome.wal_changes_applied, 2);
    assert_eq!(outcome.replicated_changes_deduplicated, 1);
    assert_eq!(
        core.events(),
        vec![
            Event::Change(Box::new(change(1))),
            Event::Change(Box::new(change(2)))
        ]
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_honours_nonzero_compaction_epoch_and_default_active_wal() {
    // A committed MANIFEST carrying a non-zero compaction_epoch (forward-compat
    // for 4c) and the default active_wal recovers normally. The active_wal
    // assert in recover() is satisfied by the de-facto fixed name.
    use crate::Manifest;

    let dir = temp_dir("compaction-epoch");
    write_snapshot(&dir, 7, &[(*b"CORE", *b"META", b"meta".to_vec())]);
    let manifest = Manifest {
        live_snapshot_seq: 7,
        active_wal_header_seq: 7,
        compaction_epoch: 42,
        active_wal: DEFAULT_WAL_FILE_NAME.to_owned(),
        archived_wal_seqs: vec![1, 3, 7],
    };
    manifest.write_atomic(&dir).unwrap();
    // Round-trip the non-zero compaction_epoch through decode.
    let read_back = Manifest::read(&dir).unwrap().unwrap();
    assert_eq!(read_back.compaction_epoch, 42);

    let core = RecordingProvider::new(*b"CORE");
    let outcome = recover(&dir, &registry(std::slice::from_ref(&core))).unwrap();
    assert!(outcome.manifest_present);
    assert_eq!(outcome.applied_snapshot_seq, 7);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_rejects_unexpected_active_wal_name() {
    // The active-WAL name is a de-facto fixed contract; a MANIFEST naming a
    // different file must fail loudly rather than silently recovering wal.log.
    use crate::Manifest;

    let dir = temp_dir("unexpected-active-wal");
    write_snapshot(&dir, 1, &[(*b"CORE", *b"META", b"meta".to_vec())]);
    let manifest = Manifest {
        live_snapshot_seq: 1,
        active_wal_header_seq: 1,
        compaction_epoch: 0,
        active_wal: "other.log".to_owned(),
        archived_wal_seqs: Vec::new(),
    };
    manifest.write_atomic(&dir).unwrap();
    let err = recover(&dir, &ProviderRegistry::new()).unwrap_err();
    assert!(
        matches!(
            err,
            PersistError::UnexpectedActiveWal { ref observed, expected }
                if observed == "other.log" && expected == DEFAULT_WAL_FILE_NAME
        ),
        "expected UnexpectedActiveWal, got {err:?}"
    );
    let _ = fs::remove_dir_all(dir);
}
