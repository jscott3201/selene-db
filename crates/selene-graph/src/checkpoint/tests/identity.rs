use selene_core::{GraphId, LabelSet, PropertyMap, db_string};
use selene_persist::{
    DEFAULT_WAL_FILE_NAME, Manifest, PersistError, SnapshotBuilder, SnapshotConfig,
};

use super::{commit_node, temp_dir, wal_graph};
use crate::{CheckpointConfig, GraphError, SharedGraph};

#[test]
fn same_sequence_compaction_conflict_is_fail_closed_until_reopen() {
    let dir = temp_dir("same-sequence-compaction");
    let graph_id = GraphId::new(91_012);
    let shared = wal_graph(&dir, graph_id);
    let tombstone = commit_node(&shared, "DeletedBeforeCheckpoint");
    let mut delete = shared.begin_write();
    delete
        .mutator()
        .delete_node(tombstone)
        .expect("node deletion stages");
    delete.commit().expect("node deletion commits");
    let first = shared
        .checkpoint(CheckpointConfig::default())
        .expect("initial checkpoint succeeds");
    assert_eq!(first.snapshot_sequence, 2);
    shared.compact().expect("compaction removes tombstone rows");

    let error = shared
        .checkpoint(CheckpointConfig::default())
        .expect_err("same-sequence physical rewrite fails closed");

    assert!(matches!(
        error,
        GraphError::Persist(PersistError::CommittedSnapshotIdentityMismatch { .. })
    ));
    let mut rejected = shared.begin_write();
    rejected
        .mutator()
        .create_node(
            LabelSet::single(db_string("RejectedAfterIdentityConflict").expect("valid label")),
            PropertyMap::new(),
        )
        .expect("mutation stages before poison is observed");
    rejected
        .commit()
        .expect_err("same-epoch committed conflict poisons the graph");
    drop(shared);

    let recovered = SharedGraph::recover(&dir, graph_id).expect("reopen restores the live epoch");
    let after = commit_node(&recovered, "AfterIdentityConflictReopen");
    assert_eq!(recovered.compaction_stats().reclaimable_nodes, 1);
    let report = recovered
        .compact()
        .expect("reopen can repeat the WAL-free compaction");
    assert_eq!(report.reclaimed_nodes, 1);
    assert_eq!(recovered.compaction_stats().reclaimable_nodes, 0);
    let repaired = recovered
        .checkpoint(CheckpointConfig::default())
        .expect("the next durable sequence checkpoints after reopen");
    assert_eq!(repaired.snapshot_sequence, 3);
    drop(recovered);

    let verified = SharedGraph::recover(&dir, graph_id).expect("repaired epoch recovers");
    assert!(verified.read().is_node_alive(after));
    assert!(!verified.read().is_node_alive(tombstone));
    assert_eq!(verified.compaction_stats().reclaimable_nodes, 0);
    drop(verified);
    std::fs::remove_dir_all(dir).expect("temp directory is removed");
}

#[test]
fn manifest_ahead_checkpoint_error_poisons_stale_graph_writer() {
    let dir = temp_dir("manifest-ahead");
    let graph_id = GraphId::new(91_013);
    let shared = wal_graph(&dir, graph_id);
    commit_node(&shared, "BeforeManifestAhead");
    Manifest {
        live_snapshot_seq: 2,
        active_wal_header_seq: 2,
        compaction_epoch: 0,
        active_wal: DEFAULT_WAL_FILE_NAME.to_owned(),
        archived_wal_seqs: vec![2],
    }
    .write_atomic(&dir)
    .expect("ahead MANIFEST publishes for the stale-writer fixture");

    let error = shared
        .checkpoint(CheckpointConfig::default())
        .expect_err("a MANIFEST ahead of the owned writer is rejected");
    assert!(matches!(
        error,
        GraphError::Persist(PersistError::WalRotationManifestAhead {
            manifest_sequence: 2,
            snapshot_sequence: 1,
        })
    ));

    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(
            LabelSet::single(db_string("RejectedAfterManifestAhead").expect("valid label")),
            PropertyMap::new(),
        )
        .expect("mutation stages before the poisoned committer rejects it");
    txn.commit()
        .expect_err("the stale committer rejects later writes");
    drop(shared);
    std::fs::remove_dir_all(dir).expect("temp directory is removed");
}

#[test]
fn foreign_committed_snapshot_conflict_poisons_graph_writer() {
    let dir = temp_dir("foreign-committed-snapshot");
    let graph_id = GraphId::new(91_014);
    let shared = wal_graph(&dir, graph_id);
    commit_node(&shared, "BeforeForeignSnapshot");
    let first = shared
        .checkpoint(CheckpointConfig::default())
        .expect("initial checkpoint succeeds");
    std::fs::remove_file(&first.snapshot_path).expect("committed snapshot removes");
    let mut foreign = SnapshotBuilder::new(SnapshotConfig {
        dir: dir.clone(),
        sequence: 1,
        compression: selene_persist::SectionCompression::None,
        fsync: true,
    });
    foreign
        .add_section(*b"TEST", *b"DATA", b"foreign-but-valid".to_vec())
        .expect("foreign section adds");
    foreign
        .finalize()
        .expect("foreign snapshot envelope publishes");

    let error = shared
        .checkpoint(CheckpointConfig::default())
        .expect_err("foreign committed snapshot fails closed");
    assert!(matches!(
        error,
        GraphError::Persist(PersistError::CommittedSnapshotIdentityMismatch { .. })
    ));

    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(
            LabelSet::single(db_string("RejectedAfterForeignSnapshot").expect("valid label")),
            PropertyMap::new(),
        )
        .expect("mutation stages before poison is observed");
    txn.commit()
        .expect_err("poisoned graph rejects later writes");
    drop(shared);
    std::fs::remove_dir_all(dir).expect("temp directory is removed");
}
