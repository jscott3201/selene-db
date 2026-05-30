use std::sync::Arc;
use std::sync::Mutex;

use selene_core::{Change, ChangeKind, ChangeKindSet, GraphId, NodeId, intern};
use selene_persist::PersistError;

use super::{append_wal, node_created, temp_dir};
use crate::{
    ChangeSubscriber, GraphError, IndexProvider, ProviderError, ProviderTag, SharedGraph, SubTag,
};

struct NoopIndexProvider {
    tag: ProviderTag,
}

impl NoopIndexProvider {
    const fn new(tag: ProviderTag) -> Self {
        Self { tag }
    }
}

impl IndexProvider for NoopIndexProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn provider_tag(&self) -> ProviderTag {
        self.tag
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

struct ErroringSubscriber {
    tag: ProviderTag,
}

struct RecordingIndexProvider {
    tag: ProviderTag,
    seen: Arc<Mutex<Vec<&'static str>>>,
}

struct RecordingSubscriber {
    tag: ProviderTag,
    seen: Arc<Mutex<Vec<&'static str>>>,
}

struct PanickingTagSubscriber;

struct PanickingFilterSubscriber {
    tag: ProviderTag,
}

struct PanickingOnChangeSubscriber {
    tag: ProviderTag,
}

impl ErroringSubscriber {
    const fn new(tag: ProviderTag) -> Self {
        Self { tag }
    }
}

impl ChangeSubscriber for ErroringSubscriber {
    fn subscriber_tag(&self) -> ProviderTag {
        self.tag
    }

    fn change_kinds(&self) -> ChangeKindSet {
        ChangeKindSet::EMPTY.with(ChangeKind::NodeDeleted)
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        Err(ProviderError::Inconsistent {
            reason: "synthetic subscriber recovery failure".to_owned(),
        })
    }
}

impl RecordingIndexProvider {
    fn new(tag: ProviderTag, seen: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self { tag, seen }
    }
}

impl IndexProvider for RecordingIndexProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

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
        self.seen.lock().unwrap().push(provider_label(change));
        Ok(())
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

impl RecordingSubscriber {
    fn new(tag: ProviderTag, seen: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self { tag, seen }
    }
}

impl ChangeSubscriber for RecordingSubscriber {
    fn subscriber_tag(&self) -> ProviderTag {
        self.tag
    }

    fn change_kinds(&self) -> ChangeKindSet {
        ChangeKindSet::EMPTY.with(ChangeKind::NodeDeleted)
    }

    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        if matches!(change, Change::NodeDeleted { .. }) {
            self.seen.lock().unwrap().push("subscriber:delete");
        }
        Ok(())
    }
}

impl ChangeSubscriber for PanickingTagSubscriber {
    fn subscriber_tag(&self) -> ProviderTag {
        panic!("synthetic subscriber tag panic")
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        Ok(())
    }
}

impl PanickingFilterSubscriber {
    const fn new(tag: ProviderTag) -> Self {
        Self { tag }
    }
}

impl ChangeSubscriber for PanickingFilterSubscriber {
    fn subscriber_tag(&self) -> ProviderTag {
        self.tag
    }

    fn change_kinds(&self) -> ChangeKindSet {
        panic!("synthetic subscriber filter panic")
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        Ok(())
    }
}

impl PanickingOnChangeSubscriber {
    const fn new(tag: ProviderTag) -> Self {
        Self { tag }
    }
}

impl ChangeSubscriber for PanickingOnChangeSubscriber {
    fn subscriber_tag(&self) -> ProviderTag {
        self.tag
    }

    fn change_kinds(&self) -> ChangeKindSet {
        ChangeKindSet::EMPTY.with(ChangeKind::NodeDeleted)
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        panic!("synthetic subscriber on_change panic")
    }
}

fn provider_label(change: &Change) -> &'static str {
    match change {
        Change::NodeCreated { .. } => "provider:create",
        Change::NodeDeleted { .. } => "provider:delete",
        Change::IndexExtensionEvent { .. } => "provider:event",
        Change::NodeUpdated { .. }
        | Change::NodePropertyRemoved { .. }
        | Change::EdgeCreated { .. }
        | Change::EdgeUpdated { .. }
        | Change::EdgePropertyRemoved { .. }
        | Change::EdgeDeleted { .. }
        | Change::NodeLabelRemoved { .. }
        | Change::NodesOfTypeTruncated { .. }
        | Change::EdgesOfTypeTruncated { .. }
        | Change::GraphReset { .. }
        | Change::SchemaChanged { .. } => "provider:other",
    }
}

fn extension_event() -> Change {
    Change::IndexExtensionEvent {
        provider: intern("ext-provider").unwrap(),
        payload: Arc::from(vec![1_u8].into_boxed_slice()),
    }
}

#[test]
fn recover_with_provider_and_empty_subscribers_replays_wal() {
    let dir = temp_dir("empty-subscribers");
    append_wal(&dir, 0, &[node_created(1)]);
    let tag = ProviderTag(*b"NOOP");
    let provider: Arc<dyn IndexProvider> = Arc::new(NoopIndexProvider::new(tag));

    let recovered =
        SharedGraph::recover_with_providers(&dir, GraphId::new(7), vec![provider], Vec::new())
            .unwrap();

    assert!(recovered.read().is_node_alive(NodeId::new(1)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recover_with_providers_rejects_unmatched_subscriber_tag() {
    let dir = temp_dir("unmatched-subscriber");
    let provider: Arc<dyn IndexProvider> = Arc::new(NoopIndexProvider::new(ProviderTag(*b"PROV")));
    let subscriber: Arc<dyn ChangeSubscriber> =
        Arc::new(ErroringSubscriber::new(ProviderTag(*b"MISS")));

    let err = match SharedGraph::recover_with_providers(
        &dir,
        GraphId::new(7),
        vec![provider],
        vec![subscriber],
    ) {
        Ok(_) => panic!("unmatched subscriber tag should fail"),
        Err(error) => error,
    };

    assert!(matches!(
        err,
        GraphError::Provider(ProviderError::Inconsistent { reason })
            if reason.contains("change subscriber tag MISS has no matching provider")
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recover_with_providers_fails_on_subscriber_error() {
    let dir = temp_dir("subscriber-error");
    append_wal(
        &dir,
        0,
        &[node_created(1), Change::NodeDeleted { id: NodeId::new(1) }],
    );
    let tag = ProviderTag(*b"SUBR");
    let provider: Arc<dyn IndexProvider> = Arc::new(NoopIndexProvider::new(tag));
    let subscriber: Arc<dyn ChangeSubscriber> = Arc::new(ErroringSubscriber::new(tag));

    let err = match SharedGraph::recover_with_providers(
        &dir,
        GraphId::new(7),
        vec![provider],
        vec![subscriber],
    ) {
        Ok(_) => panic!("subscriber error should fail recovery"),
        Err(error) => error,
    };

    let GraphError::Persist(PersistError::ProviderFailed { source, .. }) = &err else {
        panic!("expected PersistError::ProviderFailed, got {err:?}");
    };
    assert!(
        format!("{source}").contains("synthetic subscriber recovery failure"),
        "unexpected subscriber error: {source}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recover_with_providers_preserves_runtime_provider_then_subscriber_order() {
    let dir = temp_dir("subscriber-order");
    append_wal(
        &dir,
        0,
        &[
            node_created(1),
            Change::NodeDeleted { id: NodeId::new(1) },
            extension_event(),
        ],
    );
    let tag = ProviderTag(*b"ORDR");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn IndexProvider> =
        Arc::new(RecordingIndexProvider::new(tag, Arc::clone(&seen)));
    let subscriber: Arc<dyn ChangeSubscriber> =
        Arc::new(RecordingSubscriber::new(tag, Arc::clone(&seen)));

    SharedGraph::recover_with_providers(&dir, GraphId::new(7), vec![provider], vec![subscriber])
        .unwrap();

    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [
            "provider:create",
            "provider:delete",
            "provider:event",
            "subscriber:delete",
        ]
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recover_with_providers_converts_subscriber_tag_panic_to_graph_error() {
    let dir = temp_dir("subscriber-tag-panic");
    let provider: Arc<dyn IndexProvider> = Arc::new(NoopIndexProvider::new(ProviderTag(*b"PAN1")));
    let subscriber: Arc<dyn ChangeSubscriber> = Arc::new(PanickingTagSubscriber);

    let err = match SharedGraph::recover_with_providers(
        &dir,
        GraphId::new(7),
        vec![provider],
        vec![subscriber],
    ) {
        Ok(_) => panic!("subscriber tag panic should fail recovery setup"),
        Err(error) => error,
    };

    assert!(matches!(
        err,
        GraphError::Provider(ProviderError::Inconsistent { reason })
            if reason.contains("change subscriber tag lookup panicked")
                && reason.contains("synthetic subscriber tag panic")
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recover_with_providers_converts_subscriber_filter_panic_to_provider_failed() {
    let dir = temp_dir("subscriber-filter-panic");
    append_wal(&dir, 0, &[node_created(1)]);
    let tag = ProviderTag(*b"PAN2");
    let provider: Arc<dyn IndexProvider> = Arc::new(NoopIndexProvider::new(tag));
    let subscriber: Arc<dyn ChangeSubscriber> = Arc::new(PanickingFilterSubscriber::new(tag));

    let err = match SharedGraph::recover_with_providers(
        &dir,
        GraphId::new(7),
        vec![provider],
        vec![subscriber],
    ) {
        Ok(_) => panic!("subscriber filter panic should fail recovery"),
        Err(error) => error,
    };

    let GraphError::Persist(PersistError::ProviderFailed { source, .. }) = &err else {
        panic!("expected PersistError::ProviderFailed, got {err:?}");
    };
    assert!(
        format!("{source}").contains("tag/filter lookup panicked during recovery replay"),
        "unexpected subscriber panic error: {source}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recover_with_providers_converts_subscriber_on_change_panic_to_provider_failed() {
    let dir = temp_dir("subscriber-on-change-panic");
    append_wal(
        &dir,
        0,
        &[node_created(1), Change::NodeDeleted { id: NodeId::new(1) }],
    );
    let tag = ProviderTag(*b"PAN3");
    let provider: Arc<dyn IndexProvider> = Arc::new(NoopIndexProvider::new(tag));
    let subscriber: Arc<dyn ChangeSubscriber> = Arc::new(PanickingOnChangeSubscriber::new(tag));

    let err = match SharedGraph::recover_with_providers(
        &dir,
        GraphId::new(7),
        vec![provider],
        vec![subscriber],
    ) {
        Ok(_) => panic!("subscriber on_change panic should fail recovery"),
        Err(error) => error,
    };

    let GraphError::Persist(PersistError::ProviderFailed { source, .. }) = &err else {
        panic!("expected PersistError::ProviderFailed, got {err:?}");
    };
    assert!(
        format!("{source}").contains("on_change panicked during recovery replay"),
        "unexpected subscriber panic error: {source}"
    );
    let _ = std::fs::remove_dir_all(dir);
}
