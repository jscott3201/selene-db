//! Recovery-path tests for `IM_TRUNCATE` (BRIEF-150, audit Item 11).
//!
//! Invariants under test:
//! - Recovering a WAL containing ONE declarative `NodesOfTypeTruncated{L}`
//!   reconstructs the IDENTICAL alive-node/alive-edge state as recovering a WAL
//!   containing the equivalent N `NodeDeleted` + incident `EdgeDeleted`
//!   ("replay walks the store").
//! - A mock derived-state `ChangeSubscriber` receives the SAME per-row
//!   `NodeDeleted`/`EdgeDeleted` multiset during replay of the declarative
//!   variant as it would for the expanded form — the recovery anti-leak
//!   guarantee. The subscriber must never see the declarative variant.

use std::sync::{Arc, Mutex};

use selene_core::{
    Change, ChangeKind, ChangeKindSet, EdgeId, GraphId, LabelSet, NodeId, PropertyMap, intern,
};

use super::{append_wal, temp_dir};
use crate::index_provider::{IndexProvider, ProviderError, ProviderTag, SubTag};
use crate::{ChangeSubscriber, SeleneGraph, SharedGraph};

const TAG: ProviderTag = ProviderTag(*b"MOCK");

fn node_created(id: u64, label: &str) -> Change {
    Change::NodeCreated {
        id: NodeId::new(id),
        labels: LabelSet::single(intern(label).unwrap()),
        properties: PropertyMap::new(),
    }
}

fn edge_created(id: u64, source: u64, target: u64, label: &str) -> Change {
    Change::EdgeCreated {
        id: EdgeId::new(id),
        label: intern(label).unwrap(),
        source: NodeId::new(source),
        target: NodeId::new(target),
        properties: PropertyMap::new(),
    }
}

/// The shared create-section of both scenarios: 3 nodes of :L (ids 1..3), 1
/// keep node (id 4), and edges incident to the :L nodes plus a survivor edge.
fn base_creates() -> Vec<Change> {
    vec![
        node_created(1, "trec.L"),
        node_created(2, "trec.L"),
        node_created(3, "trec.L"),
        node_created(4, "trec.Keep"),
        edge_created(1, 1, 2, "trec.E1"),
        edge_created(2, 2, 3, "trec.E2"),
        edge_created(3, 3, 4, "trec.E1"),
        edge_created(4, 4, 1, "trec.E2"),
        edge_created(5, 4, 4, "trec.E1"), // survivor: both endpoints keep
    ]
}

fn assert_same_observable_state(a: &SeleneGraph, b: &SeleneGraph) {
    assert_eq!(a.node_store.alive, b.node_store.alive, "alive nodes differ");
    assert_eq!(a.edge_store.alive, b.edge_store.alive, "alive edges differ");
    assert_eq!(a.idx_label, b.idx_label, "node label index differs");
    assert_eq!(
        a.idx_edge_label, b.idx_edge_label,
        "edge label index differs"
    );
    assert_eq!(
        a.adjacency_out, b.adjacency_out,
        "outgoing adjacency differs"
    );
    assert_eq!(a.adjacency_in, b.adjacency_in, "incoming adjacency differs");
}

#[test]
fn recovery_of_declarative_truncate_matches_expanded_form() {
    // Scenario A: WAL ends with the single declarative truncate change.
    let dir_a = temp_dir("trec-declarative");
    let mut changes_a = base_creates();
    changes_a.push(Change::NodesOfTypeTruncated {
        label: intern("trec.L").unwrap(),
    });
    append_wal(&dir_a, 0, &changes_a);
    let recovered_a = SharedGraph::recover(&dir_a, GraphId::new(7)).unwrap();

    // Scenario B: WAL ends with the equivalent expanded per-row tombstones
    // (the :L nodes 1..3 and their incident edges 1,2,3,4 — NOT survivor 5).
    let dir_b = temp_dir("trec-expanded");
    let mut changes_b = base_creates();
    changes_b.push(Change::NodeDeleted { id: NodeId::new(1) });
    changes_b.push(Change::NodeDeleted { id: NodeId::new(2) });
    changes_b.push(Change::NodeDeleted { id: NodeId::new(3) });
    changes_b.push(Change::EdgeDeleted { id: EdgeId::new(1) });
    changes_b.push(Change::EdgeDeleted { id: EdgeId::new(2) });
    changes_b.push(Change::EdgeDeleted { id: EdgeId::new(3) });
    changes_b.push(Change::EdgeDeleted { id: EdgeId::new(4) });
    append_wal(&dir_b, 0, &changes_b);
    let recovered_b = SharedGraph::recover(&dir_b, GraphId::new(7)).unwrap();

    assert_same_observable_state(&recovered_a.read(), &recovered_b.read());

    // Concrete shape: only the keep node and survivor edge remain, no dangling.
    let g = recovered_a.read();
    assert!(g.is_node_alive(NodeId::new(4)));
    for id in [1_u64, 2, 3] {
        assert!(!g.is_node_alive(NodeId::new(id)), "node {id} must be dead");
    }
    assert!(g.is_edge_alive(EdgeId::new(5)), "survivor edge stays alive");
    for id in [1_u64, 2, 3, 4] {
        assert!(!g.is_edge_alive(EdgeId::new(id)), "edge {id} must be dead");
    }
    let _ = std::fs::remove_dir_all(dir_a);
    let _ = std::fs::remove_dir_all(dir_b);
}

#[test]
fn recovery_of_edge_type_truncate_matches_expanded_form() {
    let dir_a = temp_dir("trec-edge-declarative");
    let mut changes_a = base_creates();
    changes_a.push(Change::EdgesOfTypeTruncated {
        label: intern("trec.E1").unwrap(),
    });
    append_wal(&dir_a, 0, &changes_a);
    let recovered_a = SharedGraph::recover(&dir_a, GraphId::new(7)).unwrap();

    let dir_b = temp_dir("trec-edge-expanded");
    let mut changes_b = base_creates();
    // E1 edges are ids 1, 3, 5.
    changes_b.push(Change::EdgeDeleted { id: EdgeId::new(1) });
    changes_b.push(Change::EdgeDeleted { id: EdgeId::new(3) });
    changes_b.push(Change::EdgeDeleted { id: EdgeId::new(5) });
    append_wal(&dir_b, 0, &changes_b);
    let recovered_b = SharedGraph::recover(&dir_b, GraphId::new(7)).unwrap();

    assert_same_observable_state(&recovered_a.read(), &recovered_b.read());
    let g = recovered_a.read();
    assert_eq!(g.node_count(), 4, "edge truncate leaves all nodes alive");
    assert!(g.edges_with_label(&intern("trec.E1").unwrap()).is_none());
    let _ = std::fs::remove_dir_all(dir_a);
    let _ = std::fs::remove_dir_all(dir_b);
}

// --- Recovery anti-leak: subscriber must receive per-row tombstones ---

struct NoopProvider;

impl IndexProvider for NoopProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn provider_tag(&self) -> ProviderTag {
        TAG
    }
    fn read_section(&self, _sub: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }
    fn write_section(&self, _sub: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }
    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        Ok(())
    }
    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

struct TombstoneSubscriber {
    seen: Arc<Mutex<Vec<Change>>>,
}

impl ChangeSubscriber for TombstoneSubscriber {
    fn subscriber_tag(&self) -> ProviderTag {
        TAG
    }
    fn change_kinds(&self) -> ChangeKindSet {
        ChangeKindSet::EMPTY
            .with(ChangeKind::NodeDeleted)
            .with(ChangeKind::EdgeDeleted)
    }
    fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
        self.seen.lock().unwrap().push(change.clone());
        Ok(())
    }
}

#[test]
fn recovery_truncate_fans_out_per_row_tombstones_to_subscriber() {
    let dir = temp_dir("trec-subscriber-antileak");
    let mut changes = base_creates();
    changes.push(Change::NodesOfTypeTruncated {
        label: intern("trec.L").unwrap(),
    });
    append_wal(&dir, 0, &changes);

    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn IndexProvider> = Arc::new(NoopProvider);
    let subscriber: Arc<dyn ChangeSubscriber> = Arc::new(TombstoneSubscriber {
        seen: Arc::clone(&seen),
    });

    let _recovered = SharedGraph::recover_with_providers(
        &dir,
        GraphId::new(7),
        vec![provider],
        vec![subscriber],
    )
    .unwrap();

    let seen = seen.lock().unwrap();
    let deleted_nodes: std::collections::BTreeSet<NodeId> = seen
        .iter()
        .filter_map(|c| match c {
            Change::NodeDeleted { id } => Some(*id),
            _ => None,
        })
        .collect();
    let deleted_edges: std::collections::BTreeSet<EdgeId> = seen
        .iter()
        .filter_map(|c| match c {
            Change::EdgeDeleted { id } => Some(*id),
            _ => None,
        })
        .collect();
    // Every truncated :L node (1,2,3) and incident edge (1,2,3,4) must be
    // tombstoned at the subscriber during recovery — the anti-leak guarantee.
    assert_eq!(
        deleted_nodes,
        [NodeId::new(1), NodeId::new(2), NodeId::new(3)]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        "recovery subscriber missed a truncated node tombstone (derived-state leak)"
    );
    assert_eq!(
        deleted_edges,
        [
            EdgeId::new(1),
            EdgeId::new(2),
            EdgeId::new(3),
            EdgeId::new(4)
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>(),
        "recovery subscriber missed an incident-edge tombstone (derived-state leak)"
    );
    assert!(
        !seen
            .iter()
            .any(|c| matches!(c, Change::NodesOfTypeTruncated { .. })),
        "recovery subscriber must never receive the declarative truncate variant"
    );
    drop(seen);
    let _ = std::fs::remove_dir_all(dir);
}
