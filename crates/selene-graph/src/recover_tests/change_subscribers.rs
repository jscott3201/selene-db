use std::sync::Arc;

use selene_core::{Change, ChangeKind, ChangeKindSet, GraphId, NodeId};
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
