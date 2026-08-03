//! Building and recovering are different operations, and the builder now says so.
//!
//! `WalWriter::open` positions an existing WAL for **append**; it never replays
//! it. A builder starts from an empty graph. Composing the two silently layered
//! a second dataset's commits onto the first — ids restarted at 1 and collided
//! with ids the log had already allocated, so the directory then failed to
//! recover at all and the original data was unreachable.
//!
//! These tests pin that the builder refuses a populated WAL, and — the part that
//! matters operationally — that the refusal leaves the directory intact and
//! recoverable rather than half-consumed.

use super::*;

/// `SharedGraphBuilder` is not `Debug`, so `expect_err` is unavailable.
fn expect_wal_not_empty(
    result: crate::GraphResult<crate::SharedGraphBuilder>,
    context: &str,
) -> GraphError {
    let Err(error) = result else {
        panic!("{context}");
    };
    assert!(
        matches!(error, GraphError::WalNotEmpty { .. }),
        "expected WalNotEmpty, got {error:?}"
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

    let error = expect_wal_not_empty(
        SharedGraph::builder(graph_id)
            .with_wal(dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default()),
        "building over a populated WAL appends without replaying it",
    );

    assert!(
        matches!(&error, GraphError::WalNotEmpty { path } if path.ends_with(DEFAULT_WAL_FILE_NAME)),
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

    expect_wal_not_empty(
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

    expect_wal_not_empty(
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

/// The sharper positive control: a WAL whose header carries a snapshot
/// watermark but no entries — what checkpoint rotation leaves behind.
///
/// `WalWriter::open` seeds `last_sequence` from `max(header watermark, last
/// scanned entry)`, so on this file `last_sequence` is non-zero while the log
/// is empty. A guard written against `last_sequence` instead of the byte offset
/// would refuse a legitimately rotated WAL and make a checkpointed directory
/// unbuildable; the plain header-only case cannot detect that, because there
/// the watermark is zero too.
#[test]
fn builder_accepts_a_rotated_wal_with_a_watermark_but_no_entries() {
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
