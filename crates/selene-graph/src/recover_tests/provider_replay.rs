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
