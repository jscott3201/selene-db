use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use selene_core::{Change, GraphId, LabelSet, NodeId, PropertyMap, db_string};
use selene_persist::{
    DEFAULT_WAL_FILE_NAME, SyncPolicy, WalConfig, WalReader, WalWriter, snapshot_tmp_path,
};

use crate::candidate_state::MaintainedCandidateStateProvider;
use crate::index_provider::{IndexProvider, ProviderError, ProviderTag, SubTag};
use crate::{CheckpointConfig, CommitBatching, SectionCompression, SeleneGraph, SharedGraph};

const TEST_PROVIDER: [u8; 4] = *b"CKPT";
const TEST_SUB: [u8; 4] = *b"BODY";
const TEST_SUB_TAGS: &[SubTag] = &[SubTag(TEST_SUB)];

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-graph-checkpoint-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp directory is created");
    dir
}

fn wal_graph(dir: &std::path::Path, graph_id: GraphId) -> SharedGraph {
    SharedGraph::builder(graph_id)
        .with_wal(dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default())
        .expect("WAL opens")
        .with_commit_batching(CommitBatching::DEFAULT_ON)
        .build()
        .expect("graph builds")
}

fn commit_node(shared: &SharedGraph, label: &str) -> NodeId {
    let mut txn = shared.begin_write();
    let id = txn
        .mutator()
        .create_node(
            LabelSet::single(db_string(label).expect("valid label")),
            PropertyMap::new(),
        )
        .expect("node is staged");
    txn.commit().expect("node commit succeeds");
    id
}

fn active_wal_sequences(path: &std::path::Path) -> Vec<u64> {
    WalReader::open(path)
        .expect("WAL reader opens")
        .iterate(|_| true)
        .expect("WAL stream opens")
        .map(|entry| entry.expect("valid WAL entry").header.sequence)
        .collect()
}

fn snapshot_attempt_path(dir: &std::path::Path, sequence: u64, attempt: u8) -> PathBuf {
    let base = snapshot_tmp_path(dir, sequence);
    let mut name = base
        .file_name()
        .expect("snapshot temporary path has a file name")
        .to_os_string();
    name.push(format!(".{}.{attempt}", std::process::id()));
    base.with_file_name(name)
}

#[path = "tests/identity.rs"]
mod identity;

#[test]
fn checkpoint_rotates_at_ordered_boundary_and_recovery_replays_later_commit() {
    let dir = temp_dir("roundtrip");
    let wal = dir.join(DEFAULT_WAL_FILE_NAME);
    let graph_id = GraphId::new(91_001);
    let shared = wal_graph(&dir, graph_id);
    let before = commit_node(&shared, "BeforeCheckpoint");

    let first = shared
        .checkpoint(CheckpointConfig {
            compression: SectionCompression::None,
        })
        .expect("checkpoint succeeds");
    assert_eq!(first.snapshot_sequence, 2);
    assert!(first.snapshot_path.is_file());
    assert_eq!(first.rotation.snapshot_sequence(), 2);
    assert!(first.rotation.archived_path().is_some());

    let repeated = shared
        .checkpoint(CheckpointConfig {
            compression: SectionCompression::None,
        })
        .expect("repeated checkpoint reserves a fresh physical epoch");
    assert_eq!(repeated.snapshot_sequence, 3);
    assert_ne!(repeated.snapshot_path, first.snapshot_path);
    assert!(repeated.rotation.archived_path().is_some());
    assert_eq!(shared.read().meta.generation, 1);

    shared
        .compact()
        .expect("compaction orders after checkpoint");
    shared
        .rebuild_vector_indexes()
        .expect("vector maintenance orders after compaction");
    let after = commit_node(&shared, "AfterCheckpoint");
    let active = WalReader::open(&wal).expect("active WAL opens");
    assert_eq!(active.snapshot_seq(), 3);
    assert_eq!(active_wal_sequences(&wal), vec![4]);
    drop(shared);

    let recovered = SharedGraph::recover(&dir, graph_id).expect("checkpoint lineage recovers");
    let snapshot = recovered.read();
    assert!(snapshot.is_node_alive(before));
    assert!(snapshot.is_node_alive(after));
    assert_eq!(snapshot.node_count(), 2);
    drop(snapshot);
    drop(recovered);
    std::fs::remove_dir_all(dir).expect("temp directory is removed");
}

#[test]
fn checkpoint_waits_for_lower_reordered_group_before_rotating() {
    let dir = temp_dir("group-boundary");
    let graph_id = GraphId::new(91_010);
    let shared = Arc::new(wal_graph(&dir, graph_id));

    let mut txn_a = shared.begin_write();
    let a = txn_a
        .mutator()
        .create_node(
            LabelSet::single(db_string("GroupedA").expect("valid label")),
            PropertyMap::new(),
        )
        .expect("A is staged");
    let sealed_a = txn_a.seal(None, None).expect("A seals");
    let mut txn_b = shared.begin_write();
    let b = txn_b
        .mutator()
        .create_node(
            LabelSet::single(db_string("GroupedB").expect("valid label")),
            PropertyMap::new(),
        )
        .expect("B is staged");
    let sealed_b = txn_b.seal(None, None).expect("B seals");

    let checkpoint_graph = Arc::clone(&shared);
    let checkpoint =
        thread::spawn(move || checkpoint_graph.checkpoint(CheckpointConfig::default()));
    for _ in 0..2_000 {
        thread::yield_now();
    }
    let b_graph = Arc::clone(&shared);
    let b_commit = thread::spawn(move || b_graph.submit_sealed_for_test(sealed_b));
    for _ in 0..1_000 {
        thread::yield_now();
    }
    let outcome_a = shared
        .submit_sealed_for_test(sealed_a)
        .expect("A releases the contiguous group");
    let outcome_b = b_commit.join().expect("B thread joins").expect("B commits");
    assert_eq!(outcome_a.durable_at, Some(1));
    assert_eq!(outcome_b.durable_at, Some(2));

    let outcome = checkpoint
        .join()
        .expect("checkpoint thread joins")
        .expect("checkpoint runs after both lower commits");
    assert_eq!(outcome.snapshot_sequence, 3);
    assert!(shared.read().is_node_alive(a));
    assert!(shared.read().is_node_alive(b));
    assert!(active_wal_sequences(&dir.join(DEFAULT_WAL_FILE_NAME)).is_empty());
    drop(shared);

    let recovered = SharedGraph::recover(&dir, graph_id).expect("group boundary snapshot recovers");
    assert!(recovered.read().is_node_alive(a));
    assert!(recovered.read().is_node_alive(b));
    drop(recovered);
    std::fs::remove_dir_all(dir).expect("temp directory is removed");
}

#[test]
fn checkpoint_keeps_graph_generation_distinct_from_wal_sequence() {
    let dir = temp_dir("generation-sequence");
    let provider = Arc::new(
        MaintainedCandidateStateProvider::new(std::iter::empty())
            .expect("empty candidate-state provider is valid"),
    );
    let mut graph = SeleneGraph::new(GraphId::new(91_002));
    graph.meta.generation = 10;
    let writer = WalWriter::open(
        &dir.join(DEFAULT_WAL_FILE_NAME),
        WalConfig {
            sync_policy: SyncPolicy::OnFlushOnly,
            ..WalConfig::default()
        },
    )
    .expect("WAL opens");
    let shared = SharedGraph::from_graph_with_core_and_durables(
        graph,
        vec![Arc::clone(&provider) as Arc<dyn IndexProvider>],
        Vec::new(),
        Some(writer),
        None,
        CommitBatching::DEFAULT_ON,
    )
    .expect("graph builds");

    commit_node(&shared, "GenerationOne");
    assert_eq!(shared.read().meta.generation, 11);
    assert_eq!(provider.generation(), 11);
    let outcome = shared
        .checkpoint(CheckpointConfig::default())
        .expect("provider is checked against graph generation, not WAL sequence");
    assert_eq!(outcome.snapshot_sequence, 2);
    assert_eq!(shared.read().meta.generation, 11);
    assert_eq!(provider.generation(), 11);
    drop(shared);

    let recovered_provider = Arc::new(
        MaintainedCandidateStateProvider::new(std::iter::empty())
            .expect("empty candidate-state provider is valid"),
    );
    let recovered = SharedGraph::recover_with_providers(
        &dir,
        GraphId::new(91_002),
        vec![Arc::clone(&recovered_provider) as Arc<dyn IndexProvider>],
    )
    .expect("physical checkpoint sequence recovers independently of generation");
    assert_eq!(recovered.read().meta.generation, 11);
    assert_eq!(recovered_provider.generation(), 11);
    drop(recovered);
    std::fs::remove_dir_all(dir).expect("temp directory is removed");
}

#[test]
fn checkpoint_availability_errors_do_not_poison_or_wedge_commits() {
    let no_wal = SharedGraph::new(GraphId::new(91_003));
    let unavailable = no_wal
        .checkpoint(CheckpointConfig::default())
        .expect_err("in-memory graph has no checkpoint WAL");
    assert!(
        !unavailable.requires_reopen(),
        "an availability error touched nothing durable, got {unavailable:?}"
    );
    commit_node(&no_wal, "AfterNoWalError");

    let empty_dir = temp_dir("empty-wal");
    let empty = wal_graph(&empty_dir, GraphId::new(91_004));
    let empty_checkpoint = empty
        .checkpoint(CheckpointConfig::default())
        .expect("watermark lets a sequence-zero WAL checkpoint");
    assert_eq!(empty_checkpoint.snapshot_sequence, 1);
    assert_eq!(empty.read().meta.generation, 0);
    drop(empty);
    let empty = SharedGraph::recover(&empty_dir, GraphId::new(91_004))
        .expect("marker-only empty checkpoint recovers");
    assert_eq!(empty.read().meta.generation, 0);
    commit_node(&empty, "AfterSequenceZeroError");
    let after_commit = empty
        .checkpoint(CheckpointConfig::default())
        .expect("checkpoint succeeds after first durable commit");
    assert_eq!(after_commit.snapshot_sequence, 3);
    drop(empty);
    std::fs::remove_dir_all(empty_dir).expect("temp directory is removed");

    let custom_dir = temp_dir("custom-wal");
    let custom = SharedGraph::builder(GraphId::new(91_005))
        .with_wal(custom_dir.join("custom.log"), WalConfig::default())
        .expect("custom WAL opens")
        .build()
        .expect("graph builds");
    commit_node(&custom, "BeforeCustomNameError");
    custom
        .checkpoint(CheckpointConfig::default())
        .expect_err("non-conventional WAL name is rejected");
    commit_node(&custom, "AfterCustomNameError");
    assert_eq!(custom.read().meta.generation, 2);
    drop(custom);
    std::fs::remove_dir_all(custom_dir).expect("temp directory is removed");
}

struct FailureProvider {
    behavior: AtomicU8,
}

impl FailureProvider {
    fn new(behavior: u8) -> Self {
        Self {
            behavior: AtomicU8::new(behavior),
        }
    }
}

impl IndexProvider for FailureProvider {
    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(TEST_PROVIDER)
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(b"healthy".to_vec())
    }

    fn write_section_at_generation(
        &self,
        _sub_tag: SubTag,
        _generation: u64,
    ) -> Result<Vec<u8>, ProviderError> {
        match self.behavior.load(Ordering::Acquire) {
            1 => Err(ProviderError::SerializationFailed {
                reason: "synthetic checkpoint encode failure".to_owned(),
            }),
            2 => panic!("synthetic checkpoint encode panic"),
            _ => Ok(b"healthy".to_vec()),
        }
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        TEST_SUB_TAGS
    }
}

#[test]
fn provider_error_and_panic_leave_committer_retryable() {
    let dir = temp_dir("provider-failures");
    let provider = Arc::new(FailureProvider::new(1));
    let shared = SharedGraph::builder(GraphId::new(91_006))
        .with_provider(Arc::clone(&provider) as Arc<dyn IndexProvider>)
        .with_wal(dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default())
        .expect("WAL opens")
        .build()
        .expect("graph builds");
    commit_node(&shared, "BeforeProviderFailure");

    let provider_error = shared
        .checkpoint(CheckpointConfig::default())
        .expect_err("provider error fails only this checkpoint");
    // The other side of the #1087 predicate: a PREPARATION failure happens
    // before the MANIFEST protocol begins, consumes no WAL sequence, and leaves
    // the handle usable. Reclassifying it would tell an embedder to throw away
    // a perfectly good graph.
    assert!(
        !provider_error.requires_reopen(),
        "a provider encode failure is definite and retryable, got {provider_error:?}"
    );
    assert_eq!(
        active_wal_sequences(&dir.join(DEFAULT_WAL_FILE_NAME)),
        vec![1]
    );
    commit_node(&shared, "AfterProviderError");

    provider.behavior.store(2, Ordering::Release);
    let provider_panic = shared
        .checkpoint(CheckpointConfig::default())
        .expect_err("provider panic fails only this checkpoint");
    assert!(
        !provider_panic.requires_reopen(),
        "a panic caught in PREPARATION is still retryable; only a panic in the \
         watermark/rotation phase requires reopen. Got {provider_panic:?}"
    );
    assert_eq!(
        active_wal_sequences(&dir.join(DEFAULT_WAL_FILE_NAME)),
        vec![1, 2]
    );
    commit_node(&shared, "AfterProviderPanic");

    provider.behavior.store(0, Ordering::Release);
    let outcome = shared
        .checkpoint(CheckpointConfig::default())
        .expect("later checkpoint succeeds");
    assert_eq!(outcome.snapshot_sequence, 4);
    drop(shared);
    std::fs::remove_dir_all(dir).expect("temp directory is removed");
}

#[test]
fn rotation_error_poisons_until_recovery_reopens_the_graph() {
    let dir = temp_dir("rotation-error");
    let graph_id = GraphId::new(91_009);
    let shared = wal_graph(&dir, graph_id);
    let durable = commit_node(&shared, "BeforeRotationError");
    let obstructions = (0..128_u8)
        .map(|attempt| snapshot_attempt_path(&dir, 2, attempt))
        .collect::<Vec<_>>();
    for obstruction in &obstructions {
        std::fs::write(obstruction, b"occupied").expect("snapshot attempt is obstructed");
    }

    // #1087's reproduction: exhausting all 128 snapshot temporary candidates
    // fails inside the ROTATION phase, after `append_checkpoint_watermark_record`
    // has already consumed a physical sequence. The engine cannot prove which
    // side of the MANIFEST commit point it reached.
    let error = shared
        .checkpoint(CheckpointConfig::default())
        .expect_err("rotation error is reported");
    assert!(
        error.requires_reopen(),
        "an exhausted-temporary-path rotation failure poisons the committer, so \
         the caller must be told to reopen. It used to arrive as a bare \
         Persist(Io(AlreadyExists)) indistinguishable from a retryable \
         preparation failure. Got {error:?}"
    );
    assert_eq!(
        error.gqlstatus(),
        "40003",
        "the same status a commit gets on the poison exit, so an embedder has \
         one condition to handle rather than one per operation"
    );
    let reason = error.to_string();
    assert!(
        reason.contains("attempts exhausted"),
        "reclassification preserves the source error text, got {reason}"
    );

    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(
            LabelSet::single(db_string("RejectedAfterPoison").expect("valid label")),
            PropertyMap::new(),
        )
        .expect("mutation can seal before poison is observed");
    let rejected = txn
        .commit()
        .expect_err("poisoned committer rejects subsequent commit");
    assert!(rejected.requires_reopen());

    // A SECOND checkpoint already answered this way before #1087 — it goes
    // through the committer's poison gate rather than running. Pinning it here
    // records the inconsistency the fix removed: only the call that CAUSED the
    // poison used to differ.
    let second = shared
        .checkpoint(CheckpointConfig::default())
        .expect_err("the poisoned committer refuses a second checkpoint");
    assert!(second.requires_reopen());
    drop(shared);

    for obstruction in &obstructions {
        std::fs::remove_file(obstruction).expect("snapshot obstruction is removed");
    }
    let recovered = SharedGraph::recover(&dir, graph_id).expect("reopen heals writer state");
    assert!(recovered.read().is_node_alive(durable));
    assert_eq!(recovered.read().node_count(), 1);
    assert_eq!(recovered.read().meta.generation, 1);
    drop(recovered);
    std::fs::remove_dir_all(dir).expect("temp directory is removed");
}

struct RecursiveProvider {
    graph: Mutex<Weak<SharedGraph>>,
    operation: AtomicU8,
}

impl IndexProvider for RecursiveProvider {
    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(*b"RECR")
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn write_section_at_generation(
        &self,
        _sub_tag: SubTag,
        _generation: u64,
    ) -> Result<Vec<u8>, ProviderError> {
        let graph = self
            .graph
            .lock()
            .expect("recursive provider lock")
            .upgrade()
            .expect("graph is live");
        match self.operation.load(Ordering::Acquire) {
            0 => {
                graph
                    .checkpoint(CheckpointConfig::default())
                    .expect("recursive checkpoint must be rejected");
            }
            1 => {
                graph
                    .compact()
                    .expect("recursive compaction must be rejected");
            }
            2 => {
                graph
                    .rebuild_vector_indexes()
                    .expect("recursive vector rebuild must be rejected");
            }
            _ => {
                graph
                    .maintain_vector_indexes(crate::VectorIndexMaintenancePolicy::recommended())
                    .expect("recursive vector maintenance must be rejected");
            }
        }
        Ok(Vec::new())
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        TEST_SUB_TAGS
    }
}

#[test]
fn recursive_committer_operations_panic_at_guard_without_deadlocking() {
    let dir = temp_dir("recursive");
    let provider = Arc::new(RecursiveProvider {
        graph: Mutex::new(Weak::new()),
        operation: AtomicU8::new(0),
    });
    let shared = Arc::new(
        SharedGraph::builder(GraphId::new(91_007))
            .with_provider(Arc::clone(&provider) as Arc<dyn IndexProvider>)
            .with_wal(dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default())
            .expect("WAL opens")
            .build()
            .expect("graph builds"),
    );
    *provider.graph.lock().expect("recursive provider lock") = Arc::downgrade(&shared);
    commit_node(&shared, "BeforeRecursiveCheckpoint");
    shared
        .checkpoint(CheckpointConfig::default())
        .expect_err("recursive callback panic becomes a checkpoint error");
    commit_node(&shared, "AfterRecursiveCheckpoint");

    for (operation, label) in [
        (1, "AfterRecursiveCompact"),
        (2, "AfterRecursiveRebuild"),
        (3, "AfterRecursiveMaintenance"),
    ] {
        provider.operation.store(operation, Ordering::Release);
        shared
            .checkpoint(CheckpointConfig::default())
            .expect_err("recursive maintenance panic becomes a checkpoint error");
        commit_node(&shared, label);
    }
    assert_eq!(shared.read().meta.generation, 5);
    drop(shared);
    std::fs::remove_dir_all(dir).expect("temp directory is removed");
}

struct SlowProvider {
    entered: SyncSender<()>,
    release: Mutex<Receiver<()>>,
}

impl IndexProvider for SlowProvider {
    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(*b"SLOW")
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn write_section_at_generation(
        &self,
        _sub_tag: SubTag,
        _generation: u64,
    ) -> Result<Vec<u8>, ProviderError> {
        self.entered.send(()).expect("test observes callback");
        self.release
            .lock()
            .expect("slow provider lock")
            .recv()
            .expect("test releases callback");
        Ok(Vec::new())
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        TEST_SUB_TAGS
    }
}

#[test]
fn readers_continue_while_higher_writer_waits_behind_checkpoint() {
    let dir = temp_dir("concurrency");
    let (entered_tx, entered_rx) = sync_channel(1);
    let (release_tx, release_rx) = sync_channel(1);
    let provider = Arc::new(SlowProvider {
        entered: entered_tx,
        release: Mutex::new(release_rx),
    });
    let shared = Arc::new(
        SharedGraph::builder(GraphId::new(91_008))
            .with_provider(provider as Arc<dyn IndexProvider>)
            .with_wal(dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default())
            .expect("WAL opens")
            .build()
            .expect("graph builds"),
    );
    commit_node(&shared, "BeforeSlowCheckpoint");

    let checkpoint_graph = Arc::clone(&shared);
    let checkpoint =
        thread::spawn(move || checkpoint_graph.checkpoint(CheckpointConfig::default()));
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("checkpoint reaches slow provider");

    assert_eq!(shared.read().node_count(), 1, "reader stays lock-free");
    let (writer_tx, writer_rx) = sync_channel(1);
    let (writer_ready_tx, writer_ready_rx) = sync_channel(1);
    let writer_graph = Arc::clone(&shared);
    let writer = thread::spawn(move || {
        let mut txn = writer_graph.begin_write();
        let id = txn
            .mutator()
            .create_node(
                LabelSet::single(db_string("AfterSlowCheckpoint").expect("valid label")),
                PropertyMap::new(),
            )
            .expect("higher writer stages its node");
        writer_ready_tx
            .send(())
            .expect("test observes writer immediately before commit");
        let result = txn.commit().map(|_| id);
        writer_tx.send(result).expect("writer result is observed");
    });
    writer_ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("higher writer reached commit");
    assert!(
        writer_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "higher writer must wait behind checkpoint rotation"
    );

    release_tx.send(()).expect("slow provider is released");
    checkpoint
        .join()
        .expect("checkpoint thread does not panic")
        .expect("checkpoint succeeds");
    let after = writer_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("higher writer completes after checkpoint")
        .expect("higher writer commit succeeds");
    writer.join().expect("writer thread joins");
    assert!(shared.read().is_node_alive(after));
    assert_eq!(
        active_wal_sequences(&dir.join(DEFAULT_WAL_FILE_NAME)),
        vec![3],
        "higher writer receives the sequence immediately after the watermark"
    );
    drop(shared);
    std::fs::remove_dir_all(dir).expect("temp directory is removed");
}
