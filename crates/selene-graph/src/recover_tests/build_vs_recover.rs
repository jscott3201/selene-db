//! Building and recovering are different operations, and attaching now says so.
//!
//! `WalWriter::open` positions an existing WAL for **append**; it never replays
//! it. A builder starts from an empty graph. Composing the two silently layered
//! a second dataset's commits onto the first — ids restarted at 1 and collided
//! with ids the store had already allocated, so the directory then failed to
//! recover at all and the original data was unreachable.
//!
//! Three things are pinned here, and the middle one is the subtle one:
//!
//! 1. A WAL carrying entries is refused.
//! 2. So is a *checkpointed* directory, whose active WAL was reset to a bare
//!    header while its dataset moved into a snapshot. A guard that judged the
//!    WAL file alone would call that empty and bless the state every long-lived
//!    directory ends up in.
//! 3. The refusal leaves the directory intact and recoverable rather than
//!    half-consumed — a guard that locked or truncated what it declined would
//!    be worse than the corruption it prevents.
//!
//! Both attach paths are covered: the builder and `from_graph_with_wal`.

use super::*;
use crate::ExistingStoreEvidence;

/// `SharedGraphBuilder` is not `Debug`, so `expect_err` is unavailable.
fn expect_existing_store<T>(result: crate::GraphResult<T>, context: &str) -> GraphError {
    let Err(error) = result else {
        panic!("{context}");
    };
    assert!(
        matches!(error, GraphError::ExistingStore { .. }),
        "expected ExistingStore, got {error:?}"
    );
    error
}

fn node_count_after_recover(dir: &Path, graph_id: GraphId) -> usize {
    let recovered = SharedGraph::recover(dir, graph_id).unwrap();
    let count = recovered.read().node_count();
    drop(recovered);
    count
}

/// Write `nodes` nodes through a WAL-backed graph and close it.
fn seed_wal(dir: &Path, graph_id: GraphId, nodes: usize) {
    let shared = SharedGraph::builder(graph_id)
        .with_wal(dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default())
        .unwrap()
        .build()
        .unwrap();
    for index in 0..nodes {
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node(
                    LabelSet::single(db_string("build.vs.recover").unwrap()),
                    PropertyMap::from_pairs([(
                        db_string("seq").unwrap(),
                        Value::Int(index as i64),
                    )])
                    .unwrap(),
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }
    drop(shared);
}

/// The defect: a second `build` over the same directory used to succeed and
/// start appending a fresh graph's commits onto the first one's log.
#[test]
fn builder_refuses_a_wal_that_already_holds_commits() {
    let dir = temp_dir("build-over-populated-wal");
    let graph_id = GraphId::new(64);
    seed_wal(&dir, graph_id, 3);

    let error = expect_existing_store(
        SharedGraph::builder(graph_id)
            .with_wal(dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default()),
        "building over a populated WAL appends without replaying it",
    );

    assert!(
        matches!(
            &error,
            GraphError::ExistingStore { path, evidence }
                if path.ends_with(DEFAULT_WAL_FILE_NAME)
                    && *evidence == ExistingStoreEvidence::WalEntries
        ),
        "the refusal must name the WAL it declined, got {error:?}"
    );
    let _ = fs::remove_dir_all(dir);
}

/// A different `GraphId` is the shape that actually reaches a user: two graphs,
/// one directory, and before this guard the second one's commits went into the
/// first one's log.
#[test]
fn builder_refuses_a_foreign_populated_wal() {
    let dir = temp_dir("build-over-foreign-wal");
    seed_wal(&dir, GraphId::new(64), 2);

    expect_existing_store(
        SharedGraph::builder(GraphId::new(65))
            .with_wal(dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default()),
        "a foreign graph must not append onto this dataset's WAL",
    );
    let _ = fs::remove_dir_all(dir);
}

/// The operational half of the contract. Refusing is only useful if it leaves
/// the directory in the state it was found in: the writer is dropped, its lock
/// released, and every seeded row still recovers.
#[test]
fn a_refused_build_leaves_the_directory_recoverable() {
    let dir = temp_dir("refused-build-stays-recoverable");
    let graph_id = GraphId::new(64);
    seed_wal(&dir, graph_id, 3);

    expect_existing_store(
        SharedGraph::builder(graph_id)
            .with_wal(dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default()),
        "populated WAL must be refused",
    );

    assert_eq!(
        node_count_after_recover(&dir, graph_id),
        3,
        "the refusal must not consume, truncate, or lock out the WAL it declined"
    );
    let _ = fs::remove_dir_all(dir);
}

/// Positive control: a WAL that exists but carries only its header is what a
/// freshly created — or checkpoint-rotated — log looks like, and building over
/// one stays legal. Without this the guard could be "refuse every existing
/// file" and the suite would not notice.
#[test]
fn builder_accepts_an_existing_header_only_wal() {
    let dir = temp_dir("build-over-header-only-wal");
    let graph_id = GraphId::new(64);
    let path = dir.join(DEFAULT_WAL_FILE_NAME);

    // Create the WAL and close it without committing anything.
    drop(
        SharedGraph::builder(graph_id)
            .with_wal(&path, WalConfig::default())
            .unwrap()
            .build()
            .unwrap(),
    );
    assert!(path.exists(), "the WAL file must survive the first graph");

    let shared = SharedGraph::builder(graph_id)
        .with_wal(&path, WalConfig::default())
        .expect("a header-only WAL carries no commits to clobber")
        .build()
        .unwrap();

    assert_eq!(shared.read().node_count(), 0);
    drop(shared);
    let _ = fs::remove_dir_all(dir);
}

/// A checkpointed directory: the case a WAL-file-local guard gets wrong.
///
/// Rotation archives the entries and resets `wal.log` to a bare header, so the
/// dataset lives in `snapshot.N.snap` named by the MANIFEST and the active WAL
/// looks *empty*. Judging by the file alone blesses the state every long-lived
/// directory converges to, and building over it collides ids with the snapshot
/// — recovery then fails on D11 and the checkpointed data is unreachable.
#[test]
fn builder_refuses_a_checkpointed_directory_whose_wal_was_reset() {
    let dir = temp_dir("build-over-checkpointed-dir");
    let graph_id = GraphId::new(64);
    let wal = dir.join(DEFAULT_WAL_FILE_NAME);
    seed_wal(&dir, graph_id, 3);
    {
        let shared = SharedGraph::recover(&dir, graph_id).unwrap();
        shared
            .checkpoint(crate::CheckpointConfig::default())
            .unwrap();
    }

    assert_eq!(
        fs::metadata(&wal).unwrap().len(),
        selene_persist::WAL_FILE_HEADER_LEN as u64,
        "rotation must leave a header-only active WAL, or this test proves nothing"
    );

    let error = expect_existing_store(
        SharedGraph::builder(graph_id).with_wal(&wal, WalConfig::default()),
        "a checkpointed directory holds its dataset in a snapshot, not the WAL",
    );
    assert!(
        matches!(
            &error,
            GraphError::ExistingStore { evidence, .. }
                if *evidence == ExistingStoreEvidence::PublishedManifest
        ),
        "the MANIFEST is what proves it, not the WAL: {error:?}"
    );

    assert_eq!(
        node_count_after_recover(&dir, graph_id),
        3,
        "the checkpointed rows must still recover after the refusal"
    );
    let _ = fs::remove_dir_all(dir);
}

/// `from_graph_with_wal` is the sibling door onto the identical composition:
/// a caller-supplied graph attached to a WAL that is never replayed into it.
/// It must refuse what the builder refuses, or the guard is one call away from
/// being bypassed.
#[test]
fn from_graph_with_wal_refuses_a_populated_wal_too() {
    let dir = temp_dir("from-graph-over-populated-wal");
    let graph_id = GraphId::new(64);
    seed_wal(&dir, graph_id, 3);

    expect_existing_store(
        SharedGraph::from_graph_with_wal(
            crate::SeleneGraph::new(graph_id),
            dir.join(DEFAULT_WAL_FILE_NAME),
            WalConfig::default(),
        ),
        "attaching an unrelated graph to a populated WAL is the same corruption",
    );

    assert_eq!(node_count_after_recover(&dir, graph_id), 3);
    let _ = fs::remove_dir_all(dir);
}

/// The sharper positive control: a WAL whose header carries a snapshot
/// watermark but no entries, in a directory with no MANIFEST.
///
/// `WalWriter::open` seeds `last_sequence` from `max(header watermark, last
/// scanned entry)`, so `last_sequence` is non-zero while the log is empty. A
/// guard written against `last_sequence` would refuse this, and — critically —
/// so would one that treated a non-zero header watermark as proof of a live
/// snapshot. This file is byte-identical in its header to a rotated WAL; only
/// the absent MANIFEST distinguishes them, which is why the MANIFEST is the
/// evidence the guard actually uses.
#[test]
fn builder_accepts_a_watermarked_wal_with_no_published_manifest() {
    let dir = temp_dir("build-over-watermarked-empty-wal");
    let graph_id = GraphId::new(64);
    let path = dir.join(DEFAULT_WAL_FILE_NAME);
    let config = || WalConfig {
        sync_policy: SyncPolicy::OnFlushOnly,
        snapshot_seq: 7,
    };

    drop(WalWriter::open(&path, config()).unwrap());

    let shared = SharedGraph::builder(graph_id)
        .with_wal(&path, config())
        .expect("a watermarked but entry-free WAL carries no commits to clobber")
        .build()
        .unwrap();

    assert_eq!(shared.read().node_count(), 0);
    drop(shared);
    let _ = fs::remove_dir_all(dir);
}

/// Recovery must keep working on exactly the directory the builder refuses —
/// it is the operation the error tells the caller to use.
#[test]
fn recover_still_opens_what_the_builder_refuses() {
    let dir = temp_dir("recover-opens-refused-directory");
    let graph_id = GraphId::new(64);
    seed_wal(&dir, graph_id, 2);

    let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
    assert_eq!(recovered.read().node_count(), 2);

    // And it is a live writer, not a read-only view: the recovered graph
    // continues the same log rather than starting a second one.
    let mut txn = recovered.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(db_string("build.vs.recover").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
    }
    txn.commit().unwrap();
    drop(recovered);

    assert_eq!(node_count_after_recover(&dir, graph_id), 3);
    let _ = fs::remove_dir_all(dir);
}

/// The reverse direction, and the third class of directory evidence.
///
/// `SharedGraph::write_snapshot` exports a `snapshot.N.snap` with no MANIFEST
/// and no WAL. Recovery treats such a directory as a store: with no MANIFEST to
/// consult it applies the highest on-disk snapshot and seeds a fresh WAL header
/// from that sequence. So attaching an unrelated WAL beside one writes a
/// `snapshot_seq: 0` header, and recovery then cross-checks it against the
/// snapshot it just applied and refuses — the same unopenable directory,
/// reached in the opposite order from the case above.
#[test]
fn builder_refuses_a_directory_holding_a_standalone_snapshot() {
    let dir = temp_dir("build-over-standalone-snapshot");
    let graph_id = GraphId::new(64);
    let exporter = SharedGraph::builder(graph_id).build().unwrap();
    exporter
        .write_snapshot(selene_persist::SnapshotConfig {
            dir: dir.clone(),
            sequence: 7,
            compression: selene_persist::SectionCompression::None,
            fsync: false,
        })
        .unwrap();

    let error = expect_existing_store(
        SharedGraph::builder(graph_id)
            .with_wal(dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default()),
        "a snapshot with no MANIFEST is still this directory's dataset",
    );
    assert!(
        matches!(
            &error,
            GraphError::ExistingStore { evidence, .. }
                if *evidence == ExistingStoreEvidence::StandaloneSnapshot
        ),
        "the snapshot is what proves it: {error:?}"
    );
    let _ = fs::remove_dir_all(dir);
}

/// The companion positive control: recovering an export directory is the
/// operation that gets this right, seeding the fresh WAL header from the
/// applied snapshot instead of from zero. Without this, "refuse any directory
/// containing a snapshot" could be implemented in `recover` too and the suite
/// would not notice the lost capability.
#[test]
fn recover_still_opens_a_standalone_export_directory() {
    let dir = temp_dir("recover-standalone-export");
    let graph_id = GraphId::new(64);
    let exporter = SharedGraph::builder(graph_id).build().unwrap();
    {
        let mut txn = exporter.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node(
                    LabelSet::single(db_string("build.vs.recover").unwrap()),
                    PropertyMap::new(),
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }
    exporter
        .write_snapshot(selene_persist::SnapshotConfig {
            dir: dir.clone(),
            sequence: 7,
            compression: selene_persist::SectionCompression::None,
            fsync: false,
        })
        .unwrap();
    drop(exporter);

    let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
    assert_eq!(recovered.read().node_count(), 1);
    drop(recovered);
    // And it stays recoverable after the WAL it just created.
    assert_eq!(node_count_after_recover(&dir, graph_id), 1);
    let _ = fs::remove_dir_all(dir);
}
