//! GraphCommitSink integration tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use jiff::Timestamp;
use selene_core::{
    Change, GraphId, HlcTimestamp, IStr, Origin, PackLifecycleEvent, SchemaChange, intern,
};
use selene_graph::{IndexProvider, ProviderError, ProviderTag, SharedGraph, SubTag};
use selene_pack::{
    ActivationError, ActivationRegistry, GraphCommitSink, LifecycleEvent, LifecycleSink, NoopSink,
    PLACEHOLDER_CONTENT_HASH, Principal, RESERVED_LABEL_PREFIX, RESERVED_PACK_NAMESPACE, Uploaded,
};
use selene_persist::{DEFAULT_WAL_FILE_NAME, WalConfig, WalReader, WalWriter};
use serde_json::{Value, json};

struct RecordingProvider {
    tag: ProviderTag,
    changes: Mutex<Vec<Change>>,
}

impl RecordingProvider {
    fn new(tag: ProviderTag) -> Self {
        Self {
            tag,
            changes: Mutex::new(Vec::new()),
        }
    }

    fn changes(&self) -> Vec<Change> {
        self.changes.lock().expect("recording lock").clone()
    }
}

impl IndexProvider for RecordingProvider {
    fn provider_tag(&self) -> ProviderTag {
        self.tag
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        self.changes
            .lock()
            .expect("recording lock")
            .push(change.clone());
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

struct WalAppendingProvider {
    writer: Mutex<WalWriter>,
}

impl WalAppendingProvider {
    fn new(path: &Path) -> Self {
        let writer = WalWriter::open(
            path,
            WalConfig {
                fsync_every_n: 1,
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
}

impl IndexProvider for WalAppendingProvider {
    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(*b"WALP")
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

struct NoopProvider;

impl IndexProvider for NoopProvider {
    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(*b"NOOP")
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

fn ts(seconds: i64) -> Timestamp {
    Timestamp::new(seconds, 0).expect("test timestamp in range")
}

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn principal() -> Principal {
    Principal::new(istr("graph_commit_sink.principal"))
}

fn reason() -> IStr {
    istr("graph_commit_sink.reason")
}

fn manifest_bytes(pack_name: &str) -> Vec<u8> {
    serde_json::to_vec(&manifest_value(pack_name)).expect("manifest serializes")
}

fn manifest_value(pack_name: &str) -> Value {
    json!({
        "schema_version": 1,
        "pack_name": pack_name,
        "pack_version": "1.2.3",
        "content_hash": PLACEHOLDER_CONTENT_HASH,
        "procedures": [{
            "name": format!("{pack_name}.echo"),
            "tier": "graph",
            "mutability": "read",
            "input_schema": { "inline": { "type": "object" } },
            "output_schema": { "inline": { "type": "object" } },
            "capability_required": null
        }]
    })
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-pack-graph-commit-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).unwrap();
    dir
}

fn sink_with_recorder() -> (GraphCommitSink, Arc<RecordingProvider>) {
    let graph_id = GraphId::new(46);
    let provider = Arc::new(RecordingProvider::new(ProviderTag(*b"RECD")));
    let graph = Arc::new(
        SharedGraph::builder(graph_id)
            .with_provider(provider.clone())
            .build()
            .unwrap(),
    );
    (GraphCommitSink::new(graph, graph_id), provider)
}

fn lifecycle_events(changes: &[Change]) -> Vec<PackLifecycleEvent> {
    changes
        .iter()
        .filter_map(|change| match change {
            Change::SchemaChanged {
                change: SchemaChange::ProcedurePackLifecycle { event },
                ..
            } => Some(event.clone()),
            _ => None,
        })
        .collect()
}

fn stage_pack(sink: &GraphCommitSink) -> selene_pack::Staged {
    Uploaded::new(manifest_bytes("demo_pack"), principal(), ts(1))
        .validate(sink, ts(2))
        .expect("manifest validates")
        .stage(sink, ts(3))
        .expect("pack stages")
}

#[test]
fn staged_event_commits_to_graph_funnel() {
    let (sink, provider) = sink_with_recorder();
    let _staged = stage_pack(&sink);
    assert!(matches!(
        lifecycle_events(&provider.changes()).as_slice(),
        [PackLifecycleEvent::Staged { pack_name, content_hash, principal: p, at }]
            if *pack_name == istr("demo_pack")
                && *content_hash == [0_u8; 32]
                && *p == principal().as_istr()
                && *at == ts(3)
    ));
}

#[test]
fn activated_event_commits_to_graph_funnel() {
    let (sink, provider) = sink_with_recorder();
    let mut registry = ActivationRegistry::new();
    let _active = stage_pack(&sink)
        .activate(&sink, &mut registry, ts(4))
        .expect("pack activates");
    assert!(matches!(
        lifecycle_events(&provider.changes()).as_slice(),
        [_, PackLifecycleEvent::Activated { pack_name, content_hash, .. }]
            if *pack_name == istr("demo_pack") && *content_hash == [0_u8; 32]
    ));
}

#[test]
fn deprecated_event_commits_to_graph_funnel() {
    let (sink, provider) = sink_with_recorder();
    let mut registry = ActivationRegistry::new();
    let _deprecated = stage_pack(&sink)
        .activate(&sink, &mut registry, ts(4))
        .expect("pack activates")
        .deprecate(&sink, &mut registry, reason(), ts(5))
        .expect("pack deprecates");
    assert!(matches!(
        lifecycle_events(&provider.changes()).as_slice(),
        [_, _, PackLifecycleEvent::Deprecated { pack_name, reason: r, .. }]
            if *pack_name == istr("demo_pack") && *r == reason()
    ));
}

#[test]
fn disabled_event_commits_to_graph_funnel() {
    let (sink, provider) = sink_with_recorder();
    let mut registry = ActivationRegistry::new();
    let _disabled = stage_pack(&sink)
        .activate(&sink, &mut registry, ts(4))
        .expect("pack activates")
        .deprecate(&sink, &mut registry, reason(), ts(5))
        .expect("pack deprecates")
        .disable(&sink, &mut registry, ts(6))
        .expect("pack disables");
    assert!(matches!(
        lifecycle_events(&provider.changes()).as_slice(),
        [_, _, _, PackLifecycleEvent::Disabled { pack_name, principal: p, at }]
            if *pack_name == istr("demo_pack") && *p == principal().as_istr() && *at == ts(6)
    ));
}

#[test]
fn validation_failed_without_pack_name_commits_to_graph_funnel() {
    let (sink, provider) = sink_with_recorder();
    let err = Uploaded::new(b"{".to_vec(), principal(), ts(1))
        .validate(&sink, ts(2))
        .expect_err("invalid JSON fails");
    assert!(matches!(err, ActivationError::Manifest { .. }));
    assert!(matches!(
        lifecycle_events(&provider.changes()).as_slice(),
        [PackLifecycleEvent::ValidationFailed {
            pack_name: None,
            ..
        }]
    ));
}

#[test]
fn validation_failed_with_pack_name_commits_to_graph_funnel() {
    let (sink, provider) = sink_with_recorder();
    let mut value = manifest_value("demo_pack");
    value["content_hash"] = json!("sha256:bad");
    let bytes = serde_json::to_vec(&value).unwrap();
    let _err = Uploaded::new(bytes, principal(), ts(1))
        .validate(&sink, ts(2))
        .expect_err("invalid content hash fails");
    assert!(matches!(
        lifecycle_events(&provider.changes()).as_slice(),
        [PackLifecycleEvent::ValidationFailed { pack_name: Some(name), .. }]
            if *name == istr("demo_pack")
    ));
}

#[test]
fn graph_anchor_is_outer_schema_changed_graph() {
    let (sink, provider) = sink_with_recorder();
    let _staged = stage_pack(&sink);
    assert_eq!(sink.graph_anchor(), GraphId::new(46));
    assert!(matches!(
        provider.changes().as_slice(),
        [Change::SchemaChanged { graph, change: SchemaChange::ProcedurePackLifecycle { .. } }]
            if *graph == GraphId::new(46)
    ));
}

#[test]
fn principal_bytes_accessor_returns_constructor_value() {
    let graph_id = GraphId::new(46);
    let graph = Arc::new(SharedGraph::new(graph_id));
    let principal = Arc::<[u8]>::from(b"caller".as_slice());
    let sink = GraphCommitSink::new(graph, graph_id).with_principal_bytes(Arc::clone(&principal));
    assert_eq!(
        sink.principal_bytes().map(Arc::as_ref),
        Some(&principal[..])
    );
}

#[test]
fn reserved_constants_are_public() {
    assert_eq!(RESERVED_PACK_NAMESPACE, "selene");
    assert_eq!(RESERVED_LABEL_PREFIX, "selene_");
}

#[test]
fn noop_provider_smoke_test_still_commits() {
    let graph_id = GraphId::new(46);
    let graph = Arc::new(
        SharedGraph::builder(graph_id)
            .with_provider(Arc::new(NoopProvider))
            .build()
            .unwrap(),
    );
    let sink = GraphCommitSink::new(graph, graph_id);
    let _staged = stage_pack(&sink);
}

#[test]
fn lifecycle_events_are_one_per_transition() {
    let (sink, provider) = sink_with_recorder();
    let mut registry = ActivationRegistry::new();
    let _active = stage_pack(&sink)
        .activate(&sink, &mut registry, ts(4))
        .expect("pack activates");
    assert_eq!(lifecycle_events(&provider.changes()).len(), 2);
}

#[test]
fn graph_commit_sink_with_no_external_provider_still_succeeds() {
    let graph_id = GraphId::new(46);
    let graph = Arc::new(SharedGraph::new(graph_id));
    let sink = GraphCommitSink::new(graph, graph_id);
    let _staged = stage_pack(&sink);
}

#[test]
fn staged_and_activated_carry_content_hash_only_where_expected() {
    let (sink, provider) = sink_with_recorder();
    let mut registry = ActivationRegistry::new();
    let _active = stage_pack(&sink)
        .activate(&sink, &mut registry, ts(4))
        .expect("pack activates");
    let events = lifecycle_events(&provider.changes());
    assert!(matches!(
        &events[0],
        PackLifecycleEvent::Staged { content_hash, .. } if *content_hash == [0_u8; 32]
    ));
    assert!(matches!(
        &events[1],
        PackLifecycleEvent::Activated { content_hash, .. } if *content_hash == [0_u8; 32]
    ));
}

#[test]
fn deprecated_and_disabled_do_not_duplicate_content_hash() {
    let (sink, provider) = sink_with_recorder();
    let mut registry = ActivationRegistry::new();
    let _disabled = stage_pack(&sink)
        .activate(&sink, &mut registry, ts(4))
        .expect("pack activates")
        .deprecate(&sink, &mut registry, reason(), ts(5))
        .expect("pack deprecates")
        .disable(&sink, &mut registry, ts(6))
        .expect("pack disables");
    let events = lifecycle_events(&provider.changes());
    assert!(matches!(events[2], PackLifecycleEvent::Deprecated { .. }));
    assert!(matches!(events[3], PackLifecycleEvent::Disabled { .. }));
}

#[test]
fn drop_on_error_registry_hygiene_is_preserved() {
    struct RefusingSink;
    impl selene_pack::LifecycleSink for RefusingSink {
        fn record(&self, _event: &LifecycleEvent) -> Result<(), ActivationError> {
            Err(ActivationError::SinkRefused {
                detail: "refused".to_owned(),
            })
        }
    }
    let mut registry = ActivationRegistry::new();
    let staged = Uploaded::new(manifest_bytes("demo_pack"), principal(), ts(1))
        .validate(&NoopSink, ts(2))
        .unwrap()
        .stage(&NoopSink, ts(3))
        .unwrap();
    let err = staged
        .activate(&RefusingSink, &mut registry, ts(4))
        .expect_err("refusing sink aborts activation");
    assert!(matches!(err, ActivationError::SinkRefused { .. }));
    assert!(!registry.is_occupied("demo_pack"));
}

#[test]
fn wal_appending_provider_round_trips_lifecycle_events() {
    let dir = temp_dir("wal-round-trip");
    let wal_path = dir.join(DEFAULT_WAL_FILE_NAME);
    let provider = Arc::new(WalAppendingProvider::new(&wal_path));
    let graph_id = GraphId::new(46);
    let graph = Arc::new(
        SharedGraph::builder(graph_id)
            .with_provider(provider.clone())
            .build()
            .unwrap(),
    );
    let sink = GraphCommitSink::new(graph, graph_id);
    let mut registry = ActivationRegistry::new();
    let _disabled = stage_pack(&sink)
        .activate(&sink, &mut registry, ts(4))
        .expect("pack activates")
        .deprecate(&sink, &mut registry, reason(), ts(5))
        .expect("pack deprecates")
        .disable(&sink, &mut registry, ts(6))
        .expect("pack disables");
    provider.flush();

    let reader = WalReader::open(&wal_path).unwrap();
    let entries = reader
        .iterate(|_| true)
        .unwrap()
        .map(|entry| entry.unwrap().into_entry().unwrap())
        .collect::<Vec<_>>();
    let events = entries
        .iter()
        .flat_map(|entry| lifecycle_events(&entry.changes))
        .collect::<Vec<_>>();

    assert!(matches!(events[0], PackLifecycleEvent::Staged { .. }));
    assert!(matches!(events[1], PackLifecycleEvent::Activated { .. }));
    assert!(matches!(events[2], PackLifecycleEvent::Deprecated { .. }));
    assert!(matches!(events[3], PackLifecycleEvent::Disabled { .. }));
    assert_eq!(events.len(), 4);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn direct_record_disabled_event_commits_payload() {
    let (sink, provider) = sink_with_recorder();
    sink.record(&LifecycleEvent::Disabled {
        pack_name: "demo_pack".to_owned(),
        principal: principal(),
        at: ts(9),
    })
    .expect("direct record succeeds");

    assert!(matches!(
        lifecycle_events(&provider.changes()).as_slice(),
        [PackLifecycleEvent::Disabled { pack_name, at, .. }]
            if *pack_name == istr("demo_pack") && *at == ts(9)
    ));
}

#[test]
#[should_panic(expected = "GraphCommitSink anchor mismatch")]
fn graph_commit_sink_panics_on_anchor_mismatch() {
    let graph = Arc::new(SharedGraph::new(GraphId::new(46)));
    // Constructing with a different anchor than the graph identity must panic
    // (per Codex P2 — mismatched audit attribution would corrupt history).
    let _sink = GraphCommitSink::new(graph, GraphId::new(99));
}

#[test]
fn validation_failed_error_field_is_string_not_interned() {
    // Regression for Codex P2: `ValidationFailed.error` must not be admitted
    // to the global IStr interner; repeated malformed manifests would drain
    // capacity. Two different free-form error strings round-trip as distinct
    // payload bytes without contaminating the interner.
    let (sink, provider) = sink_with_recorder();
    sink.record(&LifecycleEvent::ValidationFailed {
        pack_name: None,
        principal: principal(),
        error: "offset 0: unexpected EOF".to_owned(),
        at: ts(1),
    })
    .unwrap();
    sink.record(&LifecycleEvent::ValidationFailed {
        pack_name: None,
        principal: principal(),
        error: "offset 17: trailing comma".to_owned(),
        at: ts(2),
    })
    .unwrap();
    let events = lifecycle_events(&provider.changes());
    let errors = events
        .iter()
        .filter_map(|event| match event {
            PackLifecycleEvent::ValidationFailed { error, .. } => Some(error.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        errors,
        vec![
            "offset 0: unexpected EOF".to_owned(),
            "offset 17: trailing comma".to_owned(),
        ]
    );
}

#[test]
fn graph_commit_sink_records_empty_optional_validation_pack_name() {
    let (sink, provider) = sink_with_recorder();
    sink.record(&LifecycleEvent::ValidationFailed {
        pack_name: None,
        principal: principal(),
        error: "missing pack name".to_owned(),
        at: ts(10),
    })
    .expect("direct validation failure audit succeeds");

    assert!(matches!(
        lifecycle_events(&provider.changes()).as_slice(),
        [PackLifecycleEvent::ValidationFailed { pack_name: None, error, .. }]
            if error == "missing pack name"
    ));
}
