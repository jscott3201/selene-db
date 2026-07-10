//! Recovery takeover and snapshot-only WAL lineage regressions.

use selene_persist::{Manifest, WalReader};

use super::*;
use crate::CheckpointConfig;

fn write_manifest(dir: &Path, live_snapshot_seq: u64, active_wal_header_seq: u64) {
    Manifest {
        live_snapshot_seq,
        active_wal_header_seq,
        compaction_epoch: 0,
        active_wal: DEFAULT_WAL_FILE_NAME.to_owned(),
        archived_wal_seqs: Vec::new(),
    }
    .write_atomic(dir)
    .unwrap();
}

#[test]
fn graph_recovery_owns_wal_before_replay_handoff() {
    let dir = temp_dir("writer-first-takeover");
    let wal = dir.join(DEFAULT_WAL_FILE_NAME);
    append_wal(&dir, 0, &[node_created(60)]);
    let hook_wal = wal.clone();
    super::super::set_after_persist_recovery_hook(move || {
        let error = match WalWriter::open(&hook_wal, WalConfig::default()) {
            Ok(_) => panic!("recovery must already own the WAL writer lock"),
            Err(error) => error,
        };
        assert!(matches!(error, PersistError::WriterLockHeld));
    });

    let recovered = SharedGraph::recover(&dir, GraphId::new(7)).unwrap();

    assert!(recovered.read().is_node_alive(NodeId::new(60)));
    drop(recovered);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn snapshot_only_recovery_seeds_future_wal_and_checkpoint_lineage() {
    let dir = temp_dir("snapshot-only-lineage");
    let graph_id = GraphId::new(7);
    let original = sample_shared_graph();
    let original_generation = original.read().meta.generation;
    write_snapshot(&dir, &original, 100);
    drop(original);

    let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
    assert_eq!(
        WalReader::open(&dir.join(DEFAULT_WAL_FILE_NAME))
            .unwrap()
            .snapshot_seq(),
        100
    );
    let mut txn = recovered.begin_write();
    let after = txn
        .mutator()
        .create_node(LabelSet::new(), PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();
    assert_eq!(recovered.read().meta.generation, original_generation + 1);
    let checkpoint = recovered.checkpoint(CheckpointConfig::default()).unwrap();
    assert_eq!(checkpoint.snapshot_sequence, 102);
    drop(recovered);

    let verified = SharedGraph::recover(&dir, graph_id).unwrap();
    assert!(verified.read().is_node_alive(after));
    assert_eq!(verified.read().meta.generation, original_generation + 1);
    drop(verified);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn writer_takeover_rejects_existing_wal_below_recovered_floor() {
    let dir = temp_dir("writer-below-replay-floor");
    let original = sample_shared_graph();
    write_snapshot(&dir, &original, 100);
    drop(original);
    drop(
        WalWriter::open(
            &dir.join(DEFAULT_WAL_FILE_NAME),
            WalConfig {
                sync_policy: SyncPolicy::OnFlushOnly,
                snapshot_seq: 99,
            },
        )
        .unwrap(),
    );
    write_manifest(&dir, 100, 99);

    let error = recover_err(&dir, GraphId::new(7));

    assert!(matches!(
        error,
        GraphError::Persist(PersistError::WalRecoverySequenceMismatch {
            writer_sequence: 99,
            recovered_sequence: 100,
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn writer_takeover_accepts_phase_three_lag_when_wal_reaches_snapshot() {
    let dir = temp_dir("phase-three-header-lag");
    let original = sample_shared_graph();
    write_snapshot(&dir, &original, 100);
    drop(original);
    let mut writer = WalWriter::open(
        &dir.join(DEFAULT_WAL_FILE_NAME),
        WalConfig {
            sync_policy: SyncPolicy::OnFlushOnly,
            snapshot_seq: 99,
        },
    )
    .unwrap();
    writer
        .append(HlcTimestamp::zero(), Origin::Local, None, &[])
        .unwrap();
    writer.flush().unwrap();
    assert_eq!(writer.last_sequence(), 100);
    drop(writer);
    write_manifest(&dir, 100, 99);

    let recovered = SharedGraph::recover(&dir, GraphId::new(7)).unwrap();

    assert_eq!(recovered.read().node_count(), 4);
    drop(recovered);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_snapshot_only_recovery_does_not_strand_wal_before_retry() {
    let dir = temp_dir("failed-snapshot-only-retry");
    let original = sample_shared_graph();
    write_snapshot(&dir, &original, 90);
    let corrupt_path = write_snapshot(&dir, &original, 100);
    drop(original);
    let mut bytes = fs::read(&corrupt_path).unwrap();
    *bytes.last_mut().unwrap() ^= 0xAA;
    fs::write(&corrupt_path, bytes).unwrap();

    let error = recover_err(&dir, GraphId::new(7));
    assert!(matches!(
        error,
        GraphError::Persist(PersistError::BodyHashMismatch { .. })
    ));
    assert!(
        !dir.join(DEFAULT_WAL_FILE_NAME).exists(),
        "failed snapshot verification must not publish a WAL"
    );

    fs::remove_file(corrupt_path).unwrap();
    let recovered = SharedGraph::recover(&dir, GraphId::new(7)).unwrap();
    assert_eq!(
        WalReader::open(&dir.join(DEFAULT_WAL_FILE_NAME))
            .unwrap()
            .snapshot_seq(),
        90
    );
    drop(recovered);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_wal_takeover_rejects_racing_header_lineage() {
    for manifest_present in [false, true] {
        let dir = temp_dir(if manifest_present {
            "missing-wal-racing-manifest-lineage"
        } else {
            "missing-wal-racing-legacy-lineage"
        });
        let original = sample_shared_graph();
        write_snapshot(&dir, &original, 90);
        drop(original);
        if manifest_present {
            write_manifest(&dir, 90, 90);
        }
        let racing_wal = dir.join(DEFAULT_WAL_FILE_NAME);
        super::super::set_before_missing_wal_open_hook(move || {
            let mut writer = WalWriter::open(
                &racing_wal,
                WalConfig {
                    sync_policy: SyncPolicy::OnFlushOnly,
                    snapshot_seq: 80,
                },
            )
            .unwrap();
            for sequence in 81..=90 {
                writer
                    .append(HlcTimestamp::new(sequence, 0), Origin::Local, None, &[])
                    .unwrap();
            }
            writer.flush().unwrap();
        });

        let error = recover_err(&dir, GraphId::new(7));

        assert!(matches!(
            error,
            GraphError::Persist(PersistError::WalSnapshotMismatch {
                wal_snapshot_seq: 80,
                snapshot_seq: 90,
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }
}
