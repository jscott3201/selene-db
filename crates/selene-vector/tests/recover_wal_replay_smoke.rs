//! WAL recovery adapter tests for vector extension events.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use common::graph_summary;
use selene_core::{
    Change, ChangeKind, ChangeKindSet, GraphId, HlcTimestamp, LabelSet, NodeId, Origin,
    PropertyMap, intern,
};
use selene_graph::{
    ChangeSubscriber, DEFAULT_WAL_FILE_NAME, GraphError, IndexProvider, ProviderError, ProviderTag,
    SharedGraph, SubTag, SyncPolicy, WalConfig,
};
use selene_persist::{SectionCompression, SnapshotBuilder, SnapshotConfig, WalWriter};
use selene_vector::{
    DistanceMetric, HnswConfig, HnswProvider, IvfConfig, IvfProvider, IvfStats, PqParams,
    VectorIvfUpsertV1, VectorOp, VectorUpsertPayloadV1,
};

struct WalAppendingProvider {
    writer: Mutex<WalWriter>,
}

impl WalAppendingProvider {
    fn new(path: &Path) -> Self {
        let writer = WalWriter::open(
            path,
            WalConfig {
                sync_policy: SyncPolicy::EveryN(1),
                snapshot_seq: 0,
            },
        )
        .expect("test WAL opens");
        Self {
            writer: Mutex::new(writer),
        }
    }

    fn flush(&self) {
        self.writer.lock().expect("wal lock").flush().unwrap();
    }

    fn last_sequence(&self) -> u64 {
        self.writer.lock().expect("wal lock").last_sequence()
    }

    fn rotate(&self, snapshot_seq: u64) {
        self.writer
            .lock()
            .expect("wal lock")
            .rotate(snapshot_seq)
            .unwrap();
    }
}

impl IndexProvider for WalAppendingProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(*b"WALV")
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        self.writer
            .lock()
            .expect("wal lock")
            .append(
                HlcTimestamp::zero(),
                Origin::Local,
                None,
                std::slice::from_ref(change),
            )
            .map(|_| ())
            .map_err(|error| ProviderError::Inconsistent {
                reason: error.to_string(),
            })
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

struct FailingSubscriber {
    tag: ProviderTag,
}

impl FailingSubscriber {
    const fn new(tag: ProviderTag) -> Self {
        Self { tag }
    }
}

impl ChangeSubscriber for FailingSubscriber {
    fn subscriber_tag(&self) -> ProviderTag {
        self.tag
    }

    fn change_kinds(&self) -> ChangeKindSet {
        ChangeKindSet::EMPTY.with(ChangeKind::NodeDeleted)
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        Err(ProviderError::Inconsistent {
            reason: "synthetic vector subscriber failure".to_owned(),
        })
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-vector-recover-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).unwrap();
    dir
}

fn write_snapshot(dir: &Path, graph: &SharedGraph, sequence: u64) {
    let mut builder = SnapshotBuilder::new(SnapshotConfig {
        dir: dir.to_path_buf(),
        sequence,
        compression: SectionCompression::None,
        fsync: false,
    });
    for provider in graph.index_providers() {
        for sub_tag in provider.declared_sub_tags() {
            let bytes = provider.write_section(*sub_tag).unwrap();
            builder
                .add_section(provider.provider_tag().0, sub_tag.0, bytes)
                .unwrap();
        }
    }
    let outcome = builder.finalize().unwrap();
    assert_eq!(outcome.snapshot_seq, sequence);
}

fn hnsw_config() -> HnswConfig {
    HnswConfig::new(4).expect("hnsw config is valid")
}

fn hnsw_payload(node_id: NodeId, vector: Vec<f32>, max_layer: u8) -> Arc<[u8]> {
    let payload = VectorUpsertPayloadV1 {
        op: VectorOp::Insert,
        node_id,
        vector,
        max_layer,
    }
    .encode()
    .unwrap();
    Arc::from(payload.into_boxed_slice())
}

fn ivf_config() -> IvfConfig {
    IvfConfig::with_params(
        2,
        4,
        2,
        DistanceMetric::L2,
        PqParams {
            m_subspaces: 1,
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq: false,
            use_polysemous: false,
            hamming_threshold_ratio: 0.5,
        },
        256,
    )
    .unwrap()
}

fn ivf_payload(node_id: NodeId, vector: Vec<f32>) -> Arc<[u8]> {
    let payload = VectorIvfUpsertV1 {
        op: VectorOp::Insert,
        node_id,
        vector,
    }
    .encode()
    .unwrap();
    Arc::from(payload.into_boxed_slice())
}

fn commit_extension_event(graph: &SharedGraph, provider: &str, payload: Arc<[u8]>) {
    let mut txn = graph.begin_write();
    txn.mutator()
        .extension_event(intern(provider).unwrap(), payload);
    txn.commit().expect("extension event commits");
}

fn create_nodes(graph: &SharedGraph, count: u64) {
    let mut txn = graph.begin_write();
    {
        let mut mutator = txn.mutator();
        for raw in 1..=count {
            let id = mutator
                .create_node(LabelSet::new(), PropertyMap::new())
                .expect("node create succeeds");
            assert_eq!(id, NodeId::new(raw));
        }
    }
    txn.commit().expect("node create commit succeeds");
}

fn delete_graph_node(graph: &SharedGraph, id: NodeId) {
    let mut txn = graph.begin_write();
    txn.mutator().delete_node(id).expect("node delete succeeds");
    txn.commit().expect("node delete commit succeeds");
}

fn train_ivf(provider: &IvfProvider) {
    provider.write_section(SubTag(*b"CQNT")).unwrap();
    provider.write_section(SubTag(*b"IPQB")).unwrap();
    provider.write_section(SubTag(*b"POST")).unwrap();
}

fn commit_hnsw_vectors(graph: &SharedGraph, count: u64) {
    for raw in 1..=count {
        commit_extension_event(
            graph,
            "selene-vector",
            hnsw_payload(NodeId::new(raw), vec![raw as f32, 0.0, 0.0, 0.0], 0),
        );
    }
}

fn commit_ivf_vectors(graph: &SharedGraph, count: u64) {
    for raw in 1..=count {
        commit_extension_event(
            graph,
            "selene-vector-ivf",
            ivf_payload(NodeId::new(raw), vec![raw as f32, raw as f32]),
        );
    }
}

fn index_provider<T: IndexProvider>(provider: &Arc<T>) -> Arc<dyn IndexProvider> {
    provider.clone()
}

fn change_subscriber<T: ChangeSubscriber>(subscriber: &Arc<T>) -> Arc<dyn ChangeSubscriber> {
    subscriber.clone()
}

#[test]
fn runtime_hnsw_subscriber_tombstones_deleted_node() {
    let provider = Arc::new(HnswProvider::new(hnsw_config()).unwrap());
    let graph = SharedGraph::builder(GraphId::new(9330))
        .with_provider(index_provider(&provider))
        .with_change_subscriber(change_subscriber(&provider))
        .build()
        .unwrap();
    create_nodes(&graph, 5);
    commit_hnsw_vectors(&graph, 5);
    assert!(
        provider
            .search(&[2.0, 0.0, 0.0, 0.0], 5, Some(8), None, None)
            .unwrap()
            .iter()
            .any(|(id, _)| *id == NodeId::new(2))
    );

    delete_graph_node(&graph, NodeId::new(2));

    let results = provider
        .search(&[2.0, 0.0, 0.0, 0.0], 5, Some(8), None, None)
        .unwrap();
    assert!(!results.iter().any(|(id, _)| *id == NodeId::new(2)));
}

#[test]
fn runtime_ivf_subscriber_tombstones_deleted_node() {
    let provider = Arc::new(IvfProvider::new(ivf_config()).unwrap());
    let graph = SharedGraph::builder(GraphId::new(9331))
        .with_provider(index_provider(&provider))
        .with_change_subscriber(change_subscriber(&provider))
        .build()
        .unwrap();
    create_nodes(&graph, 256);
    commit_ivf_vectors(&graph, 256);
    train_ivf(&provider);
    assert!(
        provider
            .search(&[2.0, 2.0], 256, Some(4), None, None)
            .unwrap()
            .iter()
            .any(|(id, _)| *id == NodeId::new(2))
    );

    delete_graph_node(&graph, NodeId::new(2));

    let results = provider
        .search(&[2.0, 2.0], 256, Some(4), None, None)
        .unwrap();
    assert!(!results.iter().any(|(id, _)| *id == NodeId::new(2)));
}

#[test]
fn recovery_wal_replay_subscribers_tombstone_deleted_node() {
    let dir = temp_dir("subscriber-wal-delete");
    let graph_id = GraphId::new(9332);
    let live_hnsw = Arc::new(HnswProvider::new(hnsw_config()).unwrap());
    let live_ivf = Arc::new(IvfProvider::new(ivf_config()).unwrap());
    let wal = Arc::new(WalAppendingProvider::new(&dir.join(DEFAULT_WAL_FILE_NAME)));
    let graph = SharedGraph::builder(graph_id)
        .with_provider(index_provider(&live_hnsw))
        .with_change_subscriber(change_subscriber(&live_hnsw))
        .with_provider(index_provider(&live_ivf))
        .with_change_subscriber(change_subscriber(&live_ivf))
        .with_provider(wal.clone())
        .build()
        .unwrap();
    create_nodes(&graph, 256);
    commit_hnsw_vectors(&graph, 5);
    commit_ivf_vectors(&graph, 256);
    wal.flush();
    let snapshot_seq = wal.last_sequence();
    write_snapshot(&dir, &graph, snapshot_seq);
    wal.rotate(snapshot_seq);

    delete_graph_node(&graph, NodeId::new(2));
    wal.flush();
    drop(graph);
    drop(wal);

    let recovered_hnsw = Arc::new(HnswProvider::new(hnsw_config()).unwrap());
    let recovered_ivf = Arc::new(IvfProvider::new(ivf_config()).unwrap());
    let recovered = SharedGraph::recover_with_providers(
        &dir,
        graph_id,
        vec![
            index_provider(&recovered_hnsw),
            index_provider(&recovered_ivf),
        ],
        vec![
            change_subscriber(&recovered_hnsw),
            change_subscriber(&recovered_ivf),
        ],
    )
    .unwrap();

    let hnsw_results = recovered_hnsw
        .search(&[2.0, 0.0, 0.0, 0.0], 5, Some(8), None, None)
        .unwrap();
    let ivf_results = recovered_ivf
        .search(&[2.0, 2.0], 256, Some(4), None, None)
        .unwrap();
    assert!(!hnsw_results.iter().any(|(id, _)| *id == NodeId::new(2)));
    assert!(!ivf_results.iter().any(|(id, _)| *id == NodeId::new(2)));
    assert!(!recovered.read().is_node_alive(NodeId::new(2)));
    drop(recovered);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recovery_snapshot_rebuild_preserves_vector_tombstones() {
    let dir = temp_dir("subscriber-snapshot-delete");
    let graph_id = GraphId::new(9333);
    let live_hnsw = Arc::new(HnswProvider::new(hnsw_config()).unwrap());
    let live_ivf = Arc::new(IvfProvider::new(ivf_config()).unwrap());
    let wal = Arc::new(WalAppendingProvider::new(&dir.join(DEFAULT_WAL_FILE_NAME)));
    let graph = SharedGraph::builder(graph_id)
        .with_provider(index_provider(&live_hnsw))
        .with_change_subscriber(change_subscriber(&live_hnsw))
        .with_provider(index_provider(&live_ivf))
        .with_change_subscriber(change_subscriber(&live_ivf))
        .with_provider(wal.clone())
        .build()
        .unwrap();
    create_nodes(&graph, 256);
    commit_hnsw_vectors(&graph, 5);
    commit_ivf_vectors(&graph, 256);
    delete_graph_node(&graph, NodeId::new(2));
    wal.flush();
    let snapshot_seq = wal.last_sequence();
    write_snapshot(&dir, &graph, snapshot_seq);
    wal.rotate(snapshot_seq);
    drop(graph);
    drop(wal);

    let recovered_hnsw = Arc::new(HnswProvider::new(hnsw_config()).unwrap());
    let recovered_ivf = Arc::new(IvfProvider::new(ivf_config()).unwrap());
    let recovered = SharedGraph::recover_with_providers(
        &dir,
        graph_id,
        vec![
            index_provider(&recovered_hnsw),
            index_provider(&recovered_ivf),
        ],
        vec![
            change_subscriber(&recovered_hnsw),
            change_subscriber(&recovered_ivf),
        ],
    )
    .unwrap();

    let hnsw_results = recovered_hnsw
        .search(&[2.0, 0.0, 0.0, 0.0], 5, Some(8), None, None)
        .unwrap();
    let ivf_results = recovered_ivf
        .search(&[2.0, 2.0], 256, Some(4), None, None)
        .unwrap();
    assert!(!hnsw_results.iter().any(|(id, _)| *id == NodeId::new(2)));
    assert!(!ivf_results.iter().any(|(id, _)| *id == NodeId::new(2)));
    assert!(!recovered.read().is_node_alive(NodeId::new(2)));
    drop(recovered);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recovery_subscriber_error_fails_vector_recovery() {
    let dir = temp_dir("subscriber-error");
    let graph_id = GraphId::new(9334);
    let live_hnsw = Arc::new(HnswProvider::new(hnsw_config()).unwrap());
    let wal = Arc::new(WalAppendingProvider::new(&dir.join(DEFAULT_WAL_FILE_NAME)));
    let graph = SharedGraph::builder(graph_id)
        .with_provider(index_provider(&live_hnsw))
        .with_provider(wal.clone())
        .build()
        .unwrap();
    create_nodes(&graph, 1);
    delete_graph_node(&graph, NodeId::new(1));
    wal.flush();
    drop(graph);
    drop(wal);

    let recovered_hnsw = Arc::new(HnswProvider::new(hnsw_config()).unwrap());
    let failing: Arc<dyn ChangeSubscriber> =
        Arc::new(FailingSubscriber::new(ProviderTag(*b"VECT")));
    let err = match SharedGraph::recover_with_providers(
        &dir,
        graph_id,
        vec![index_provider(&recovered_hnsw)],
        vec![failing],
    ) {
        Ok(_) => panic!("subscriber error should fail recovery"),
        Err(error) => error,
    };

    let GraphError::Persist(selene_persist::PersistError::ProviderFailed { source, .. }) = &err
    else {
        panic!("expected provider failure, got {err:?}");
    };
    assert!(
        format!("{source}").contains("synthetic vector subscriber failure"),
        "unexpected subscriber error: {source}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_replays_index_extension_event_to_hnsw_provider() {
    let dir = temp_dir("hnsw");
    let graph_id = GraphId::new(9319);
    let live_provider = Arc::new(HnswProvider::new(hnsw_config()).unwrap());
    let wal = Arc::new(WalAppendingProvider::new(&dir.join(DEFAULT_WAL_FILE_NAME)));
    let graph = SharedGraph::builder(graph_id)
        .with_provider(live_provider.clone())
        .with_provider(wal.clone())
        .build()
        .unwrap();

    commit_extension_event(
        &graph,
        "selene-vector",
        hnsw_payload(NodeId::new(1), vec![1.0, 0.0, 0.0, 0.0], 0),
    );
    commit_extension_event(
        &graph,
        "selene-vector",
        hnsw_payload(NodeId::new(2), vec![0.0, 1.0, 0.0, 0.0], 0),
    );
    wal.flush();
    // Release the test fixture's WalAppendingProvider before recover — the
    // core durable provider now reopens the WAL file under an exclusive
    // writer lock and would otherwise observe WriterLockHeld.
    drop(graph);
    drop(wal);

    let replay_provider = Arc::new(HnswProvider::new(hnsw_config()).unwrap());
    let recovered = SharedGraph::recover_with_providers(
        &dir,
        graph_id,
        vec![index_provider(&replay_provider)],
        Vec::new(),
    )
    .unwrap();
    assert_eq!(recovered.read().node_count(), 0);
    assert!(
        recovered
            .index_provider_by_tag(ProviderTag(*b"VECT"))
            .is_some()
    );

    assert_eq!(
        graph_summary(&replay_provider.snapshot()),
        graph_summary(&live_provider.snapshot())
    );
    let results = replay_provider
        .search(&[0.0, 1.0, 0.0, 0.0], 1, Some(8), None, None)
        .unwrap();
    assert_eq!(results.first().map(|(id, _)| *id), Some(NodeId::new(2)));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_replays_index_extension_event_to_ivf_provider() {
    let dir = temp_dir("ivf");
    let graph_id = GraphId::new(9320);
    let live_provider = Arc::new(IvfProvider::new(ivf_config()).unwrap());
    let wal = Arc::new(WalAppendingProvider::new(&dir.join(DEFAULT_WAL_FILE_NAME)));
    let graph = SharedGraph::builder(graph_id)
        .with_provider(live_provider.clone())
        .with_provider(wal.clone())
        .build()
        .unwrap();

    commit_extension_event(
        &graph,
        "selene-vector-ivf",
        ivf_payload(NodeId::new(1), vec![1.0, 0.0]),
    );
    commit_extension_event(
        &graph,
        "selene-vector-ivf",
        ivf_payload(NodeId::new(2), vec![0.0, 1.0]),
    );
    wal.flush();
    // See HNSW twin: release the test WalAppendingProvider before recover.
    drop(graph);
    drop(wal);

    let replay_provider = Arc::new(IvfProvider::new(ivf_config()).unwrap());
    let recovered = SharedGraph::recover_with_providers(
        &dir,
        graph_id,
        vec![index_provider(&replay_provider)],
        Vec::new(),
    )
    .unwrap();
    assert_eq!(recovered.read().node_count(), 0);
    assert!(
        recovered
            .index_provider_by_tag(ProviderTag(*b"IVFP"))
            .is_some()
    );

    assert_eq!(
        replay_provider.snapshot().len(),
        live_provider.snapshot().len()
    );
    assert_eq!(
        replay_provider.ivf_stats().unwrap(),
        Some(IvfStats::Deferred {
            observed_vectors: 2,
            required: 256,
        })
    );
    assert_eq!(
        replay_provider.ivf_stats().unwrap(),
        live_provider.ivf_stats().unwrap()
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_with_providers_applies_vector_snapshot_sections_and_post_snapshot_wal() {
    let dir = temp_dir("snapshot-wal");
    let graph_id = GraphId::new(9321);
    let live_hnsw = Arc::new(HnswProvider::new(hnsw_config()).unwrap());
    let live_ivf = Arc::new(IvfProvider::new(ivf_config()).unwrap());
    let wal = Arc::new(WalAppendingProvider::new(&dir.join(DEFAULT_WAL_FILE_NAME)));
    let graph = SharedGraph::builder(graph_id)
        .with_provider(live_hnsw.clone())
        .with_provider(live_ivf.clone())
        .with_provider(wal.clone())
        .build()
        .unwrap();

    commit_extension_event(
        &graph,
        "selene-vector",
        hnsw_payload(NodeId::new(1), vec![1.0, 0.0, 0.0, 0.0], 0),
    );
    commit_extension_event(
        &graph,
        "selene-vector-ivf",
        ivf_payload(NodeId::new(1), vec![1.0, 0.0]),
    );
    wal.flush();
    let snapshot_seq = wal.last_sequence();
    write_snapshot(&dir, &graph, snapshot_seq);
    wal.rotate(snapshot_seq);

    commit_extension_event(
        &graph,
        "selene-vector",
        hnsw_payload(NodeId::new(2), vec![0.0, 1.0, 0.0, 0.0], 0),
    );
    commit_extension_event(
        &graph,
        "selene-vector-ivf",
        ivf_payload(NodeId::new(2), vec![0.0, 1.0]),
    );
    wal.flush();
    drop(graph);
    drop(wal);

    let recovered_hnsw = Arc::new(HnswProvider::new(hnsw_config()).unwrap());
    let recovered_ivf = Arc::new(IvfProvider::new(ivf_config()).unwrap());
    let recovered = SharedGraph::recover_with_providers(
        &dir,
        graph_id,
        vec![
            index_provider(&recovered_hnsw),
            index_provider(&recovered_ivf),
        ],
        Vec::new(),
    )
    .unwrap();

    assert_eq!(
        graph_summary(&recovered_hnsw.snapshot()),
        graph_summary(&live_hnsw.snapshot())
    );
    assert_eq!(recovered_ivf.snapshot().len(), live_ivf.snapshot().len());
    assert_eq!(
        recovered_ivf.ivf_stats().unwrap(),
        live_ivf.ivf_stats().unwrap()
    );
    let results = recovered_hnsw
        .search(&[0.0, 1.0, 0.0, 0.0], 1, Some(8), None, None)
        .unwrap();
    assert_eq!(results.first().map(|(id, _)| *id), Some(NodeId::new(2)));

    commit_extension_event(
        &recovered,
        "selene-vector",
        hnsw_payload(NodeId::new(3), vec![0.0, 0.0, 1.0, 0.0], 0),
    );
    commit_extension_event(
        &recovered,
        "selene-vector-ivf",
        ivf_payload(NodeId::new(3), vec![0.5, 0.5]),
    );
    let results = recovered_hnsw
        .search(&[0.0, 0.0, 1.0, 0.0], 1, Some(8), None, None)
        .unwrap();
    assert_eq!(results.first().map(|(id, _)| *id), Some(NodeId::new(3)));
    assert_eq!(recovered_ivf.snapshot().len(), 3);
    drop(recovered);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_with_providers_rejects_duplicate_provider_tags() {
    let dir = temp_dir("duplicate-provider");
    let first = Arc::new(HnswProvider::new(hnsw_config()).unwrap());
    let second = Arc::new(HnswProvider::new(hnsw_config()).unwrap());

    let err = match SharedGraph::recover_with_providers(
        &dir,
        GraphId::new(9322),
        vec![index_provider(&first), index_provider(&second)],
        Vec::new(),
    ) {
        Ok(_) => panic!("duplicate provider tags should fail"),
        Err(error) => error,
    };

    assert!(matches!(
        err,
        GraphError::Provider(ProviderError::Inconsistent { reason })
            if reason.contains("duplicate provider tag VECT")
    ));
    let _ = fs::remove_dir_all(dir);
}
