use selene_core::{GraphId, LabelSet, PropertyMap, db_string};
use selene_persist::{
    DEFAULT_WAL_FILE_NAME, Manifest, PersistError, SectionCompression, SnapshotBuilder,
    SnapshotConfig,
};

use super::{active_wal_sequences, commit_node, temp_dir, wal_graph};
use crate::{CheckpointConfig, GraphError, SharedGraph};

#[test]
fn wal_free_compaction_checkpoints_at_a_fresh_physical_sequence() {
    let dir = temp_dir("wal-free-compaction");
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
    assert_eq!(first.snapshot_sequence, 3);
    let report = shared.compact().expect("compaction removes tombstone rows");
    assert_eq!(report.reclaimed_nodes, 1);
    assert_eq!(shared.compaction_stats().reclaimable_nodes, 0);

    let compacted = shared
        .checkpoint(CheckpointConfig::default())
        .expect("WAL-free compaction receives a fresh checkpoint watermark");
    assert_eq!(compacted.snapshot_sequence, 4);
    assert_eq!(shared.read().meta.generation, 2);
    drop(shared);

    let recovered = SharedGraph::recover(&dir, graph_id).expect("dense checkpoint recovers");
    assert!(!recovered.read().is_node_alive(tombstone));
    assert_eq!(recovered.read().meta.generation, 2);
    assert_eq!(recovered.compaction_stats().reclaimable_nodes, 0);
    let after = commit_node(&recovered, "AfterCompactedCheckpoint");
    assert_eq!(recovered.read().meta.generation, 3);
    drop(recovered);

    let verified = SharedGraph::recover(&dir, graph_id).expect("later commit recovers");
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
fn next_target_collision_consumes_marker_without_poisoning_graph() {
    let dir = temp_dir("next-target-collision");
    let graph_id = GraphId::new(91_015);
    let shared = wal_graph(&dir, graph_id);
    let before = commit_node(&shared, "BeforeTargetCollision");
    let mut foreign = SnapshotBuilder::new(SnapshotConfig {
        dir: dir.clone(),
        sequence: 2,
        compression: SectionCompression::None,
        fsync: true,
    });
    foreign
        .add_section(*b"TEST", *b"DATA", b"foreign-target".to_vec())
        .expect("foreign section adds");
    foreign.finalize().expect("foreign target publishes");

    let error = shared
        .checkpoint(CheckpointConfig::default())
        .expect_err("foreign next target fails closed");
    assert!(matches!(
        error,
        GraphError::Persist(PersistError::ArtifactIdentityMismatch { .. })
    ));
    assert_eq!(
        active_wal_sequences(&dir.join(DEFAULT_WAL_FILE_NAME)),
        vec![1, 2]
    );

    let after = commit_node(&shared, "AfterTargetCollision");
    let repaired = shared
        .checkpoint(CheckpointConfig::default())
        .expect("later fresh target checkpoints without reopening");
    assert_eq!(repaired.snapshot_sequence, 4);
    assert_eq!(shared.read().meta.generation, 2);
    drop(shared);

    let recovered = SharedGraph::recover(&dir, graph_id).expect("later epoch recovers");
    assert!(recovered.read().is_node_alive(before));
    assert!(recovered.read().is_node_alive(after));
    assert_eq!(recovered.read().meta.generation, 2);
    drop(recovered);
    std::fs::remove_dir_all(dir).expect("temp directory is removed");
}

#[test]
fn repeated_checkpoint_with_different_compression_gets_a_fresh_epoch() {
    let dir = temp_dir("compression-change");
    let graph_id = GraphId::new(91_014);
    let shared = wal_graph(&dir, graph_id);
    let node = commit_node(&shared, "BeforeCompressionChange");
    let first = shared
        .checkpoint(CheckpointConfig {
            compression: SectionCompression::None,
        })
        .expect("initial checkpoint succeeds");
    let second = shared
        .checkpoint(CheckpointConfig::default())
        .expect("compression change publishes under a fresh watermark");
    assert_eq!(first.snapshot_sequence, 2);
    assert_eq!(second.snapshot_sequence, 3);
    assert_ne!(first.snapshot_path, second.snapshot_path);
    assert_eq!(shared.read().meta.generation, 1);
    drop(shared);

    let recovered = SharedGraph::recover(&dir, graph_id).expect("new compression epoch recovers");
    assert!(recovered.read().is_node_alive(node));
    assert_eq!(recovered.read().meta.generation, 1);
    drop(recovered);
    std::fs::remove_dir_all(dir).expect("temp directory is removed");
}
