use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use selene_core::{Change, GraphId, HlcTimestamp, Origin, PropertyMap, intern};
use selene_persist::{
    DEFAULT_WAL_FILE_NAME, SectionCompression, SnapshotBuilder, SnapshotConfig, SyncPolicy,
    WalConfig, WalWriter, snapshot_path,
};

use super::*;
use crate::SharedGraph;

fn label(name: &str) -> IStr {
    intern(name).unwrap()
}

fn current_spec() -> (CandidateStateSpec, IStr, IStr, IStr, IStr) {
    let name = label("current");
    let doc = label("MemoryFact");
    let superseded = label("SUPERSEDED_BY");
    let contradicts = label("CONTRADICTS");
    let spec = CandidateStateSpec::new(name.clone())
        .require_label(doc.clone())
        .exclude_outgoing(superseded.clone())
        .exclude_incoming(contradicts.clone());
    (spec, name, doc, superseded, contradicts)
}

fn provider_with(spec: CandidateStateSpec) -> Arc<MaintainedCandidateStateProvider> {
    Arc::new(MaintainedCandidateStateProvider::new([spec]).unwrap())
}

fn candidate_nodes(provider: &MaintainedCandidateStateProvider, name: &IStr) -> Vec<NodeId> {
    provider
        .candidate_set(name)
        .expect("candidate set is configured")
        .into_nodes()
}

#[test]
fn provider_tracks_label_and_edge_exclusions_through_commits() {
    let (spec, name, doc, superseded, contradicts) = current_spec();
    let provider = provider_with(spec);
    let shared = SharedGraph::builder(GraphId::new(81_001))
        .with_provider(provider.clone() as Arc<dyn IndexProvider>)
        .build()
        .unwrap();

    let (active, stale, unresolved, non_doc) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let active = mutator
            .create_node(LabelSet::single(doc.clone()), PropertyMap::new())
            .unwrap();
        let stale = mutator
            .create_node(LabelSet::single(doc.clone()), PropertyMap::new())
            .unwrap();
        let unresolved = mutator
            .create_node(LabelSet::single(doc), PropertyMap::new())
            .unwrap();
        let non_doc = mutator
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        (active, stale, unresolved, non_doc)
    };

    assert_eq!(
        candidate_nodes(&provider, &name),
        vec![active, stale, unresolved]
    );
    assert!(!provider.contains(&name, non_doc));

    let (stale_edge, contradiction_edge) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let stale_edge = mutator
            .create_edge(superseded, stale, active, PropertyMap::new())
            .unwrap();
        let contradiction_edge = mutator
            .create_edge(contradicts, non_doc, unresolved, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        (stale_edge, contradiction_edge)
    };

    assert_eq!(candidate_nodes(&provider, &name), vec![active]);

    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        mutator.delete_edge(stale_edge).unwrap();
        mutator.delete_edge(contradiction_edge).unwrap();
        txn.commit().unwrap();
    }

    assert_eq!(
        candidate_nodes(&provider, &name),
        vec![active, stale, unresolved]
    );
}

#[test]
fn provider_can_rebuild_from_existing_graph_snapshot() {
    let (spec, name, doc, superseded, _) = current_spec();
    let shared = SharedGraph::new(GraphId::new(81_002));
    let (active, stale) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let active = mutator
            .create_node(LabelSet::single(doc.clone()), PropertyMap::new())
            .unwrap();
        let stale = mutator
            .create_node(LabelSet::single(doc), PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(superseded, stale, active, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        (active, stale)
    };

    let provider = MaintainedCandidateStateProvider::from_graph([spec], shared.read().as_ref())
        .expect("provider rebuild succeeds");

    assert_eq!(candidate_nodes(&provider, &name), vec![active]);
    assert!(!provider.contains(&name, stale));
}

#[test]
fn provider_snapshot_and_wal_replay_preserve_delete_reverse_state() {
    let (spec, name, doc, superseded, _) = current_spec();
    let provider = provider_with(spec.clone());
    let shared = SharedGraph::builder(GraphId::new(81_003))
        .with_provider(provider.clone() as Arc<dyn IndexProvider>)
        .build()
        .unwrap();
    let (active, stale, stale_edge) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let active = mutator
            .create_node(LabelSet::single(doc.clone()), PropertyMap::new())
            .unwrap();
        let stale = mutator
            .create_node(LabelSet::single(doc), PropertyMap::new())
            .unwrap();
        let edge = mutator
            .create_edge(superseded, stale, active, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        (active, stale, edge)
    };
    assert_eq!(candidate_nodes(&provider, &name), vec![active]);

    let dir = temp_dir("candidate-state");
    write_snapshot_all_providers(&dir, &shared, 1);
    append_wal(&dir, 1, &[Change::EdgeDeleted { id: stale_edge }]);

    let recovered_provider = provider_with(spec);
    let recovered = SharedGraph::recover_with_providers(
        &dir,
        GraphId::new(81_003),
        vec![recovered_provider.clone() as Arc<dyn IndexProvider>],
    )
    .unwrap();

    assert!(!recovered.read().is_edge_alive(stale_edge));
    assert_eq!(
        candidate_nodes(&recovered_provider, &name),
        vec![active, stale]
    );
}

#[test]
fn provider_wal_replay_expands_declarative_edge_truncate_from_state() {
    let (spec, name, doc, superseded, _) = current_spec();
    let provider = provider_with(spec.clone());
    let shared = SharedGraph::builder(GraphId::new(81_005))
        .with_provider(provider.clone() as Arc<dyn IndexProvider>)
        .build()
        .unwrap();
    let (active, stale, stale_edge) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let active = mutator
            .create_node(LabelSet::single(doc.clone()), PropertyMap::new())
            .unwrap();
        let stale = mutator
            .create_node(LabelSet::single(doc), PropertyMap::new())
            .unwrap();
        let edge = mutator
            .create_edge(superseded.clone(), stale, active, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        (active, stale, edge)
    };
    assert_eq!(candidate_nodes(&provider, &name), vec![active]);

    let dir = temp_dir("candidate-state-edge-truncate");
    write_snapshot_all_providers(&dir, &shared, 1);
    append_wal(
        &dir,
        1,
        &[Change::EdgesOfTypeTruncated { label: superseded }],
    );

    let recovered_provider = provider_with(spec);
    let recovered = SharedGraph::recover_with_providers(
        &dir,
        GraphId::new(81_005),
        vec![recovered_provider.clone() as Arc<dyn IndexProvider>],
    )
    .unwrap();

    assert!(!recovered.read().is_edge_alive(stale_edge));
    assert_eq!(
        candidate_nodes(&recovered_provider, &name),
        vec![active, stale]
    );
}

#[test]
fn provider_wal_replay_expands_declarative_node_truncate_from_state() {
    let (spec, name, doc, superseded, _) = current_spec();
    let provider = provider_with(spec.clone());
    let shared = SharedGraph::builder(GraphId::new(81_006))
        .with_provider(provider.clone() as Arc<dyn IndexProvider>)
        .build()
        .unwrap();
    let (active, stale, stale_edge) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let active = mutator
            .create_node(LabelSet::single(doc.clone()), PropertyMap::new())
            .unwrap();
        let stale = mutator
            .create_node(LabelSet::single(doc.clone()), PropertyMap::new())
            .unwrap();
        let edge = mutator
            .create_edge(superseded, stale, active, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        (active, stale, edge)
    };
    assert_eq!(candidate_nodes(&provider, &name), vec![active]);

    let dir = temp_dir("candidate-state-node-truncate");
    write_snapshot_all_providers(&dir, &shared, 1);
    append_wal(&dir, 1, &[Change::NodesOfTypeTruncated { label: doc }]);

    let recovered_provider = provider_with(spec);
    let recovered = SharedGraph::recover_with_providers(
        &dir,
        GraphId::new(81_006),
        vec![recovered_provider.clone() as Arc<dyn IndexProvider>],
    )
    .unwrap();

    assert!(!recovered.read().is_node_alive(active));
    assert!(!recovered.read().is_node_alive(stale));
    assert!(!recovered.read().is_edge_alive(stale_edge));
    assert!(candidate_nodes(&recovered_provider, &name).is_empty());
}

#[test]
fn duplicate_spec_names_are_rejected() {
    let name = label("current");
    let err = match MaintainedCandidateStateProvider::new([
        CandidateStateSpec::new(name.clone()),
        CandidateStateSpec::new(name),
    ]) {
        Ok(_) => panic!("duplicate specs must be rejected"),
        Err(error) => error,
    };

    assert!(matches!(err, ProviderError::Inconsistent { .. }));
}

#[test]
fn provider_rejects_snapshot_spec_drift() {
    let (spec, _, _, _, _) = current_spec();
    let provider = provider_with(spec);
    let bytes = provider
        .write_section(SubTag(CANDIDATE_STATE_SUB))
        .expect("snapshot section writes");
    let drifted = provider_with(CandidateStateSpec::new(label("other")));

    let err = drifted
        .read_section(SubTag(CANDIDATE_STATE_SUB), &bytes)
        .expect_err("spec drift must fail recovery");

    assert!(matches!(err, ProviderError::InvalidPayload { .. }));
}

#[test]
fn provider_rejects_snapshot_dangling_tracked_edge() {
    let (spec, _, doc, superseded, _) = current_spec();
    let provider = provider_with(spec.clone());
    let snapshot = CandidateStateSnapshot {
        version: SNAPSHOT_VERSION,
        specs: vec![spec],
        node_labels: vec![(NodeId::new(1), LabelSet::single(doc))],
        edges: vec![(
            EdgeId::new(1),
            TrackedEdge {
                label: superseded,
                source: NodeId::new(2),
                target: NodeId::new(1),
            },
        )],
    };
    let bytes = postcard::to_stdvec(&snapshot).unwrap();

    let err = provider
        .read_section(SubTag(CANDIDATE_STATE_SUB), &bytes)
        .expect_err("dangling tracked edge must fail recovery");

    assert!(matches!(err, ProviderError::InvalidPayload { .. }));
}

#[test]
fn provider_canonicalizes_public_spec_label_vectors() {
    let name = label("current");
    let doc = label("MemoryFact");
    let superseded = label("SUPERSEDED_BY");
    let other = label("OTHER");
    let spec = CandidateStateSpec {
        name: name.clone(),
        required_label: Some(doc.clone()),
        exclude_outgoing: vec![other, superseded.clone(), superseded.clone()],
        exclude_incoming: Vec::new(),
    };
    let provider = provider_with(spec);
    let shared = SharedGraph::builder(GraphId::new(81_004))
        .with_provider(provider.clone() as Arc<dyn IndexProvider>)
        .build()
        .unwrap();
    let (active, stale) = {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let active = mutator
            .create_node(LabelSet::single(doc.clone()), PropertyMap::new())
            .unwrap();
        let stale = mutator
            .create_node(LabelSet::single(doc), PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(superseded, stale, active, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
        (active, stale)
    };

    assert_eq!(candidate_nodes(&provider, &name), vec![active]);
    assert!(!provider.contains(&name, stale));
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-graph-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    dir
}

fn write_snapshot_all_providers(dir: &Path, shared: &SharedGraph, sequence: u64) {
    let mut builder = SnapshotBuilder::new(SnapshotConfig {
        dir: dir.to_path_buf(),
        sequence,
        compression: SectionCompression::None,
        fsync: false,
    });
    for provider in shared.index_providers() {
        for sub in provider.declared_sub_tags() {
            let bytes = provider.write_section(*sub).unwrap();
            builder
                .add_section(provider.provider_tag().0, sub.0, bytes)
                .unwrap();
        }
    }
    let outcome = builder.finalize().unwrap();
    assert_eq!(outcome.snapshot_seq, sequence);
    assert!(snapshot_path(dir, sequence).exists());
}

fn append_wal(dir: &Path, snapshot_seq: u64, changes: &[Change]) {
    let mut writer = WalWriter::open(
        &dir.join(DEFAULT_WAL_FILE_NAME),
        WalConfig {
            sync_policy: SyncPolicy::EveryN(1),
            snapshot_seq,
        },
    )
    .unwrap();
    writer
        .append(HlcTimestamp::zero(), Origin::Local, None, changes)
        .unwrap();
    writer.flush().unwrap();
}
