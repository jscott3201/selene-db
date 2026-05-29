//! Cross-storage snapshot-time compaction contract (D23, BRIEF-Item-4b).
//!
//! Deletion leaves dead rows in every storage provider's columns. Compaction
//! reclaims them at snapshot time: the CORE graph (the liveness authority)
//! computes the set of surviving external ids ([`LiveIdSet`]) and hands it to
//! every other storage provider so they can drop derived state for ids the
//! graph no longer holds. External `NodeId`/`EdgeId` are stable across
//! compaction (D22); only internal row layout and row-keyed derived state move.
//!
//! 4b ships this trait (the forward contract) plus the CORE compaction
//! *mechanism* in [`crate::compaction`]. The CORE graph does not implement this
//! trait — it is the authority that *produces* the [`LiveIdSet`] via
//! [`crate::compaction::compact_core`], rather than a downstream consumer of
//! one. Downstream storage providers (e.g. `selene-vector` in BRIEF-Item-4d,
//! future `selene-timeseries` / `selene-rdf`) implement this trait to compact
//! their own state against the graph's live set. The snapshot publisher
//! (BRIEF-Item-4c) runs all compactors atomically under the MANIFEST epoch.

use std::collections::HashSet;

use selene_core::{EdgeId, NodeId};

use crate::index_provider::{ProviderError, ProviderTag};

/// The external ids that survive a compaction — the cross-storage liveness
/// contract every [`StorageCompactor`] keys against.
///
/// Membership is by *external* id (stable across compaction), never by internal
/// `RowIndex`, so a downstream provider that keys its own state by `NodeId` /
/// `EdgeId` can reclaim everything absent from these sets without knowing the
/// graph's row layout.
#[derive(Clone, Debug, Default)]
pub struct LiveIdSet {
    /// Surviving (alive) external node ids.
    pub nodes: HashSet<NodeId>,
    /// Surviving (alive) external edge ids.
    pub edges: HashSet<EdgeId>,
}

impl LiveIdSet {
    /// Construct an empty live set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `id` survives the compaction.
    #[must_use]
    pub fn contains_node(&self, id: NodeId) -> bool {
        self.nodes.contains(&id)
    }

    /// Whether `id` survives the compaction.
    #[must_use]
    pub fn contains_edge(&self, id: EdgeId) -> bool {
        self.edges.contains(&id)
    }
}

/// What one provider reclaimed during a compaction pass.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CompactionReport {
    /// Node rows (dead + hole) dropped.
    pub reclaimed_nodes: u64,
    /// Edge rows (dead + hole) dropped.
    pub reclaimed_edges: u64,
}

/// Snapshot-time storage compaction for a single storage provider (D23).
///
/// Implementors drop derived/persisted state for every id NOT in `live`,
/// preserving everything keyed by a surviving external id. The receiver is
/// `&self` (interior mutability / staged output) to match the other extension
/// provider traits ([`crate::IndexProvider`], [`crate::ChangeSubscriber`]); the
/// snapshot publisher (4c) is responsible for ordering and atomic publication.
pub trait StorageCompactor: Send + Sync + 'static {
    /// The provider tag this compactor compacts state for.
    fn provider_tag(&self) -> ProviderTag;

    /// Reclaim storage for everything not in `live`, returning what was dropped.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] if the provider's state is structurally
    /// inconsistent with `live` (e.g. it holds an id the graph never had).
    fn compact_for_snapshot(&self, live: &LiveIdSet) -> Result<CompactionReport, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    // The contract must stay dyn-compatible so 4c can hold
    // `Vec<Arc<dyn StorageCompactor>>` across providers.
    #[allow(dead_code)]
    fn assert_dyn_safe(_: &dyn StorageCompactor) {}

    #[test]
    fn live_id_set_membership() {
        let mut live = LiveIdSet::new();
        live.nodes.insert(NodeId::new(5));
        live.edges.insert(EdgeId::new(3));
        assert!(live.contains_node(NodeId::new(5)));
        assert!(!live.contains_node(NodeId::new(6)));
        assert!(live.contains_edge(EdgeId::new(3)));
        assert!(!live.contains_edge(EdgeId::new(1)));
    }
}
