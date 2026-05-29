//! Runtime tests for `DROP GRAPH` factory-reset (BRIEF-152, audit Item 10).
//!
//! Invariants under test:
//! - factory_reset wipes ALL nodes/edges INCLUDING untyped/arbitrary-label rows
//!   (uses live_nodes/live_edges, not per-type) — observationally equal to
//!   `MATCH (n) DETACH DELETE n` + full schema drop.
//! - The schema resets to open (bound_type -> None): a previously closed GG02
//!   graph becomes open and accepts a previously-invalid insert.
//! - Exactly ONE `Change::GraphReset` is written regardless of N (O(1) WAL).
//! - A vector-like `ChangeSubscriber` receives per-row NodeDeleted/EdgeDeleted
//!   tombstones for every removed row — the anti-leak guarantee — and never the
//!   declarative GraphReset variant.
//! - Idempotent: a second DROP GRAPH on an empty + open graph is a clean no-op.

use std::sync::{Arc, Mutex};

use selene_core::{Change, ChangeKind, ChangeKindSet, GraphId, NodeId, PropertyMap, Value, intern};

use super::*;
use crate::index_provider::{IndexProvider, ProviderError, ProviderTag, SubTag};
use crate::{ChangeSubscriber, SharedGraph};

const TAG: ProviderTag = ProviderTag(*b"MOCK");

fn prop(key: &str, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(intern(key).unwrap(), value)]).unwrap()
}

/// Open (GG01) fixture mixing TYPED-looking and UNTYPED nodes/edges, plus an
/// edge between two untyped nodes whose label is not a declared type. A per-type
/// truncate would never reach the untyped rows; factory_reset must.
fn open_fixture() -> SharedGraph {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    {
        let mut m = txn.mutator();
        let labelled = intern("fr.Labelled").unwrap();
        let other = intern("fr.Other").unwrap();
        let a = m
            .create_node(LabelSet::single(labelled), prop("k", Value::Int(0)))
            .unwrap();
        let b = m
            .create_node(
                LabelSet::from_iter([labelled, other]),
                prop("k", Value::Int(1)),
            )
            .unwrap();
        // An UNTYPED node: empty label set, legal under GG01.
        let untyped = m
            .create_node(LabelSet::new(), prop("k", Value::Int(2)))
            .unwrap();
        let e = intern("fr.E").unwrap();
        m.create_edge(e, a, b, PropertyMap::new()).unwrap();
        // Edge touching the untyped node — must still be wiped.
        m.create_edge(e, b, untyped, PropertyMap::new()).unwrap();
        m.create_edge(e, untyped, a, PropertyMap::new()).unwrap();
    }
    txn.commit().unwrap();
    shared
}

#[test]
fn factory_reset_wipes_all_nodes_and_edges_including_untyped() {
    let shared = open_fixture();
    assert_eq!(shared.read().node_count(), 3);
    assert_eq!(shared.read().edge_count(), 3);

    let mut txn = shared.begin_write();
    txn.mutator().factory_reset().unwrap();
    txn.commit().unwrap();

    let g = shared.read();
    assert_eq!(g.node_count(), 0, "all nodes wiped, incl untyped");
    assert_eq!(g.edge_count(), 0, "all edges wiped, incl untyped-incident");
    assert!(g.idx_label.values().all(roaring::RoaringBitmap::is_empty));
    assert!(
        g.idx_edge_label
            .values()
            .all(roaring::RoaringBitmap::is_empty),
        "edge-label index buckets cleared"
    );
    assert!(g.adjacency_out.is_empty(), "outgoing adjacency cleared");
    assert!(g.adjacency_in.is_empty(), "incoming adjacency cleared");
}

#[test]
fn factory_reset_writes_exactly_one_change_regardless_of_n() {
    let shared = open_fixture();
    let mut txn = shared.begin_write();
    txn.mutator().factory_reset().unwrap();
    let outcome = txn.commit().unwrap();
    // 3 nodes + 3 edges removed, but the persisted changeset carries exactly ONE
    // declarative change (the O(1)-WAL invariant).
    assert_eq!(outcome.changes.len(), 1, "exactly one persisted change");
    assert!(matches!(outcome.changes[0], Change::GraphReset {}));
}

#[test]
fn factory_reset_resets_closed_graph_to_open() {
    use crate::{NodeTypeDef, PropertyTypeDef, ValidationMode};
    use selene_core::PropertyValueType;

    // A closed (GG02) graph with a strict Person type requiring `name`.
    let graph_type = GraphTypeDef {
        name: intern("fr.person.graph").unwrap(),
        node_types: vec![NodeTypeDef {
            name: intern("Person").unwrap(),
            key_labels: LabelSet::single(intern("Person").unwrap()),
            properties: vec![PropertyTypeDef {
                name: intern("name").unwrap(),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: true,
                immutable: false,
                default: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: vec![],
    };
    let shared = SharedGraph::builder(GraphId::new(2))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap();
    assert!(shared.is_closed());

    // Seed a valid Person, then factory-reset.
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(intern("Person").unwrap()),
                prop("name", Value::String(intern("Alice").unwrap())),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    {
        let mut txn = shared.begin_write();
        txn.mutator().factory_reset().unwrap();
        txn.commit().unwrap();
    }

    let g = shared.read();
    assert_eq!(g.node_count(), 0);
    assert!(g.meta.bound_type.is_none(), "schema reset to open (None)");
    assert!(!shared.is_closed(), "graph is open after DROP GRAPH");

    // A node WITHOUT the required `name` would have been a GG02 violation before
    // the reset; under the now-open graph it commits cleanly.
    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(
            LabelSet::single(intern("Person").unwrap()),
            PropertyMap::new(),
        )
        .unwrap();
    txn.commit()
        .expect("open graph accepts a node that violated the dropped GG02 type");
    assert_eq!(shared.read().node_count(), 1);
}

#[test]
fn factory_reset_on_empty_open_graph_is_clean_noop() {
    let shared = SharedGraph::new(GraphId::new(3));
    // First reset on an empty graph: still emits one GraphReset (observable
    // no-op), never an error.
    let mut txn = shared.begin_write();
    txn.mutator().factory_reset().unwrap();
    let outcome = txn.commit().unwrap();
    assert_eq!(
        outcome.changes.len(),
        1,
        "empty reset still writes GraphReset"
    );
    assert!(matches!(outcome.changes[0], Change::GraphReset {}));
    assert_eq!(shared.read().node_count(), 0);
    assert!(shared.read().meta.bound_type.is_none());

    // Double DROP GRAPH succeeds again.
    let mut txn = shared.begin_write();
    txn.mutator().factory_reset().unwrap();
    txn.commit().expect("double DROP GRAPH is idempotent");
}

// --- Anti-leak: a vector-like subscriber must receive per-row tombstones ---

struct RecordingProvider;

impl IndexProvider for RecordingProvider {
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
fn factory_reset_fans_out_per_row_tombstones_to_subscriber() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn IndexProvider> = Arc::new(RecordingProvider);
    let subscriber: Arc<dyn ChangeSubscriber> = Arc::new(TombstoneSubscriber {
        seen: Arc::clone(&seen),
    });

    let shared = SharedGraph::builder(GraphId::new(4))
        .with_provider(provider)
        .with_change_subscriber(subscriber)
        .build()
        .unwrap();

    let (expected_nodes, expected_edges) = {
        let mut txn = shared.begin_write();
        let (n, e) = {
            let mut m = txn.mutator();
            let labelled = intern("fr.sub.L").unwrap();
            let a = m
                .create_node(LabelSet::single(labelled), PropertyMap::new())
                .unwrap();
            let b = m
                .create_node(LabelSet::single(labelled), PropertyMap::new())
                .unwrap();
            let untyped = m.create_node(LabelSet::new(), PropertyMap::new()).unwrap();
            let edge = intern("fr.sub.E").unwrap();
            let e0 = m.create_edge(edge, a, b, PropertyMap::new()).unwrap();
            let e1 = m.create_edge(edge, b, untyped, PropertyMap::new()).unwrap();
            (vec![a, b, untyped], vec![e0, e1])
        };
        txn.commit().unwrap();
        (
            n.into_iter().collect::<std::collections::BTreeSet<_>>(),
            e.into_iter().collect::<std::collections::BTreeSet<_>>(),
        )
    };

    seen.lock().unwrap().clear();
    {
        let mut txn = shared.begin_write();
        txn.mutator().factory_reset().unwrap();
        txn.commit().unwrap();
    }

    let seen = seen.lock().unwrap();
    let deleted_nodes: std::collections::BTreeSet<NodeId> = seen
        .iter()
        .filter_map(|c| match c {
            Change::NodeDeleted { id } => Some(*id),
            _ => None,
        })
        .collect();
    let deleted_edges: std::collections::BTreeSet<_> = seen
        .iter()
        .filter_map(|c| match c {
            Change::EdgeDeleted { id } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(
        deleted_nodes, expected_nodes,
        "subscriber missed a node tombstone (vector leak), incl untyped"
    );
    assert_eq!(
        deleted_edges, expected_edges,
        "subscriber missed an edge tombstone (vector leak)"
    );
    assert!(
        !seen.iter().any(|c| matches!(c, Change::GraphReset {})),
        "subscriber must never receive the declarative GraphReset variant"
    );
}

#[test]
fn factory_reset_matches_detach_delete_plus_schema_drop_observable_state() {
    // factory_reset must leave the SAME alive bitmaps + indexes as manually
    // deleting every node and clearing the bound type.
    let reset = open_fixture();
    let manual = open_fixture();

    {
        let mut txn = reset.begin_write();
        txn.mutator().factory_reset().unwrap();
        txn.commit().unwrap();
    }
    {
        let mut txn = manual.begin_write();
        let ids: Vec<NodeId> = txn
            .read()
            .live_nodes()
            .iter()
            .map(|row| NodeId::new(u64::from(row) + 1))
            .collect();
        {
            let mut m = txn.mutator();
            for id in ids {
                m.delete_node(id).unwrap();
            }
        }
        txn.commit().unwrap();
    }

    let a = reset.read();
    let b = manual.read();
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
