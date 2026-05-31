//! Recovery tests for `recover_with_providers` (index-provider WAL replay).

use std::sync::Arc;

use selene_core::{Change, GraphId, NodeId};

use super::{append_wal, node_created, temp_dir};
use crate::{IndexProvider, ProviderError, ProviderTag, SharedGraph, SubTag};

struct NoopIndexProvider {
    tag: ProviderTag,
}

impl NoopIndexProvider {
    const fn new(tag: ProviderTag) -> Self {
        Self { tag }
    }
}

impl IndexProvider for NoopIndexProvider {
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

#[test]
fn recover_with_providers_replays_wal() {
    let dir = temp_dir("provider-replay");
    append_wal(&dir, 0, &[node_created(1)]);
    let tag = ProviderTag(*b"NOOP");
    let provider: Arc<dyn IndexProvider> = Arc::new(NoopIndexProvider::new(tag));

    let recovered =
        SharedGraph::recover_with_providers(&dir, GraphId::new(7), vec![provider]).unwrap();

    assert!(recovered.read().is_node_alive(NodeId::new(1)));
    let _ = std::fs::remove_dir_all(dir);
}

/// An index provider whose `on_change` always errors, to prove WAL-replay
/// provider errors propagate out of `recover_with_providers` (GRAPH-39).
struct FailingIndexProvider {
    tag: ProviderTag,
}

impl FailingIndexProvider {
    const fn new(tag: ProviderTag) -> Self {
        Self { tag }
    }
}

impl IndexProvider for FailingIndexProvider {
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
        Err(ProviderError::Inconsistent {
            reason: "synthetic index-provider on_change failure".to_owned(),
        })
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

#[test]
fn recover_with_providers_propagates_index_on_change_error() {
    // GRAPH-39: WAL replay drives every registered provider's `on_change` (via the
    // `IndexRecoveryProvider` wrapper) for each replayed entry. An index provider
    // that errors there must abort recovery with the boxed error surfaced — not be
    // silently swallowed (which would leave the rebuilt index inconsistent with
    // the recovered graph). Only the always-Ok Noop provider was exercised before.
    let dir = temp_dir("provider-replay-fail");
    append_wal(&dir, 0, &[node_created(1)]);
    let provider: Arc<dyn IndexProvider> =
        Arc::new(FailingIndexProvider::new(ProviderTag(*b"FAIL")));

    let err = match SharedGraph::recover_with_providers(&dir, GraphId::new(7), vec![provider]) {
        Ok(_) => panic!("recovery must fail when an index provider's on_change errors"),
        Err(error) => error,
    };
    let crate::GraphError::Persist(selene_persist::PersistError::ProviderFailed {
        provider,
        source,
        ..
    }) = &err
    else {
        panic!("expected PersistError::ProviderFailed, got {err:?}");
    };
    assert_eq!(
        *provider, *b"FAIL",
        "the failing provider's tag is surfaced"
    );
    assert!(
        format!("{source}").contains("synthetic index-provider on_change failure"),
        "the boxed provider error must surface verbatim, got: {source}",
    );
    let _ = std::fs::remove_dir_all(dir);
}
