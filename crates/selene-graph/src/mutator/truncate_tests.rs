//! Runtime tests for `IM_TRUNCATE` bulk truncate (BRIEF-150, audit Item 11).
//!
//! Invariants under test:
//! - TRUNCATE NODE TYPE :L is observationally identical to
//!   `MATCH (n:L) DETACH DELETE n` (same final graph state, no dangling edges).
//! - Exactly ONE declarative change is written regardless of N (O(1) WAL).
//! - A `ChangeSubscriber` (e.g. a derived-state provider) receives per-row tombstones
//!   for every truncated node/edge — the anti-leak guarantee.
//! - Empty/absent label is a clean no-op; double-truncate is idempotent.

use std::sync::{Arc, Mutex};

use selene_core::{Change, ChangeKind, ChangeKindSet, GraphId, NodeId, PropertyMap, Value, intern};

use super::*;
use crate::index_provider::{IndexProvider, ProviderError, ProviderTag, SubTag};
use crate::{ChangeSubscriber, SharedGraph};

const TAG: ProviderTag = ProviderTag(*b"MOCK");

fn prop(key: &str, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(intern(key).unwrap(), value)]).unwrap()
}

/// Build a fixture: K nodes of label :L (each carrying an extra label and a
/// property index target), incident edges of TWO edge types between them and to
/// some non-:L nodes, plus a non-:L node/edge that must survive truncation.
///
/// Returns the shared graph (uncommitted txn already committed) so callers can
/// open a fresh txn and truncate or detach-delete.
fn fixture() -> SharedGraph {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    {
        let mut m = txn.mutator();
        let l = intern("trunc.L").unwrap();
        let other = intern("trunc.Other").unwrap();
        let keep = intern("trunc.Keep").unwrap();
        // 4 nodes of :L (rows 0..3), 1 keep node (row 4).
        let l0 = m
            .create_node(LabelSet::single(l), prop("k", Value::Int(0)))
            .unwrap();
        let l1 = m
            .create_node(LabelSet::from_iter([l, other]), prop("k", Value::Int(1)))
            .unwrap();
        let l2 = m
            .create_node(LabelSet::single(l), prop("k", Value::Int(2)))
            .unwrap();
        let l3 = m
            .create_node(LabelSet::single(l), prop("k", Value::Int(3)))
            .unwrap();
        let keep_node = m
            .create_node(LabelSet::single(keep), prop("k", Value::Int(9)))
            .unwrap();
        let e1 = intern("trunc.E1").unwrap();
        let e2 = intern("trunc.E2").unwrap();
        // Incident edges of two types: L->L, L->keep, keep->L.
        m.create_edge(e1, l0, l1, PropertyMap::new()).unwrap();
        m.create_edge(e2, l1, l2, PropertyMap::new()).unwrap();
        m.create_edge(e1, l3, keep_node, PropertyMap::new())
            .unwrap();
        m.create_edge(e2, keep_node, l0, PropertyMap::new())
            .unwrap();
        // Edge entirely among survivors must be untouched.
        m.create_edge(e1, keep_node, keep_node, PropertyMap::new())
            .unwrap();
    }
    txn.commit().unwrap();
    shared
}

/// Compare the observable state of two graphs that should be identical after a
/// truncate vs DETACH DELETE of the same set.
fn assert_same_observable_state(a: &crate::SeleneGraph, b: &crate::SeleneGraph) {
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
    // Surviving node labels/properties must match row-for-row.
    for row in a.node_store.alive.iter() {
        let row = row as usize;
        assert_eq!(
            a.node_store.labels.get(row),
            b.node_store.labels.get(row),
            "surviving node row {row} labels differ"
        );
        assert_eq!(
            a.node_store.properties.get(row),
            b.node_store.properties.get(row),
            "surviving node row {row} properties differ"
        );
    }
}

#[test]
fn truncate_node_type_matches_detach_delete_observable_state() {
    let truncated = fixture();
    let detached = fixture();
    let l = intern("trunc.L").unwrap();

    // TRUNCATE NODE TYPE :L
    {
        let mut txn = truncated.begin_write();
        txn.mutator().truncate_node_type(l).unwrap();
        txn.commit().unwrap();
    }
    // MATCH (n:L) DETACH DELETE n — resolve matched ids from the label index,
    // then per-row delete_node (which cascades incident edges).
    {
        let mut txn = detached.begin_write();
        let matched: Vec<NodeId> = txn
            .read()
            .nodes_with_label(&l)
            .map(|bitmap| {
                bitmap
                    .iter()
                    .map(|row| NodeId::new(u64::from(row) + 1))
                    .collect()
            })
            .unwrap_or_default();
        {
            let mut m = txn.mutator();
            for id in matched {
                m.delete_node(id).unwrap();
            }
        }
        txn.commit().unwrap();
    }

    assert_same_observable_state(&truncated.read(), &detached.read());
    // No node of :L survives, and no dangling edge: every alive edge's
    // endpoints must be alive nodes.
    let g = truncated.read();
    assert!(g.nodes_with_label(&l).is_none(), "no :L nodes survive");
    for row in g.edge_store.alive.iter() {
        let row = row as usize;
        let source = *g.edge_store.source.get(row).unwrap();
        let target = *g.edge_store.target.get(row).unwrap();
        assert!(
            g.is_node_alive(source),
            "edge row {row} has dangling source"
        );
        assert!(
            g.is_node_alive(target),
            "edge row {row} has dangling target"
        );
    }
}

#[test]
fn truncate_writes_exactly_one_change_regardless_of_n() {
    let shared = fixture();
    let l = intern("trunc.L").unwrap();
    let mut txn = shared.begin_write();
    txn.mutator().truncate_node_type(l).unwrap();
    let outcome = txn.commit().unwrap();
    // 4 :L nodes + 4 incident edges removed, but the persisted changeset carries
    // exactly ONE declarative change (the O(1)-WAL invariant).
    assert_eq!(
        outcome.changes.len(),
        1,
        "expected exactly one persisted change"
    );
    assert!(matches!(
        outcome.changes[0],
        Change::NodesOfTypeTruncated { label } if label == l
    ));
}

#[test]
fn truncate_edge_type_writes_one_change_and_removes_only_that_type() {
    let shared = fixture();
    let e1 = intern("trunc.E1").unwrap();
    let mut txn = shared.begin_write();
    txn.mutator().truncate_edge_type(e1).unwrap();
    let outcome = txn.commit().unwrap();
    assert_eq!(outcome.changes.len(), 1);
    assert!(matches!(
        outcome.changes[0],
        Change::EdgesOfTypeTruncated { label } if label == e1
    ));
    let g = shared.read();
    assert!(g.edges_with_label(&e1).is_none(), "all E1 edges removed");
    // E2 edges and all nodes untouched.
    let e2 = intern("trunc.E2").unwrap();
    assert!(g.edges_with_label(&e2).is_some(), "E2 edges survive");
    assert_eq!(g.node_count(), 5, "truncate edge type leaves nodes intact");
}

#[test]
fn truncate_absent_label_is_clean_noop() {
    let shared = fixture();
    let absent = intern("trunc.NoSuchLabel").unwrap();
    let mut txn = shared.begin_write();
    txn.mutator().truncate_node_type(absent).unwrap();
    let outcome = txn.commit().unwrap();
    assert!(outcome.changes.is_empty(), "absent label writes no change");
    assert_eq!(shared.read().node_count(), 5);
}

#[test]
fn double_truncate_is_idempotent() {
    let shared = fixture();
    let l = intern("trunc.L").unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator().truncate_node_type(l).unwrap();
        txn.commit().unwrap();
    }
    let mut txn = shared.begin_write();
    txn.mutator().truncate_node_type(l).unwrap();
    let outcome = txn.commit().unwrap();
    // Second truncate finds no :L rows (index removed), so it is a no-op.
    assert!(outcome.changes.is_empty(), "second truncate writes nothing");
}

// --- Anti-leak: a subscriber must receive per-row tombstones ---

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

/// Mirrors the derived-state provider contract: subscribes only to NodeDeleted/
/// EdgeDeleted and records every tombstone it receives.
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
fn truncate_fans_out_per_row_tombstones_to_subscriber() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn IndexProvider> = Arc::new(RecordingProvider);
    let subscriber: Arc<dyn ChangeSubscriber> = Arc::new(TombstoneSubscriber {
        seen: Arc::clone(&seen),
    });

    // Build the fixture inside a subscriber-attached SharedGraph.
    let shared = build_fixture_with_subscriber(provider, subscriber);
    let l = intern("trunc.sub.L").unwrap();

    // Capture the matched ids BEFORE truncation so we know exactly what the
    // subscriber must observe.
    let (expected_nodes, expected_edges) = {
        let g = shared.read();
        let nodes: Vec<NodeId> = g
            .nodes_with_label(&l)
            .map(|bitmap| {
                bitmap
                    .iter()
                    .map(|row| NodeId::new(u64::from(row) + 1))
                    .collect()
            })
            .unwrap_or_default();
        let mut edges = std::collections::BTreeSet::new();
        for id in &nodes {
            if let Some(entry) = g.outgoing_edges(*id) {
                edges.extend(entry.iter().map(|edge| edge.edge_id));
            }
            if let Some(entry) = g.incoming_edges(*id) {
                edges.extend(entry.iter().map(|edge| edge.edge_id));
            }
        }
        (nodes, edges)
    };
    assert!(!expected_nodes.is_empty());
    assert!(!expected_edges.is_empty());

    seen.lock().unwrap().clear();
    {
        let mut txn = shared.begin_write();
        txn.mutator().truncate_node_type(l).unwrap();
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
    // The subscriber must have received a tombstone for EVERY truncated node
    // and incident edge — the anti-leak guarantee. It must NEVER see the
    // declarative truncate variant (filtered out of fan-out by kind).
    assert_eq!(
        deleted_nodes,
        expected_nodes
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        "subscriber missed a truncated node tombstone (derived-state leak)"
    );
    assert_eq!(
        deleted_edges, expected_edges,
        "subscriber missed an incident-edge tombstone (derived-state leak)"
    );
    assert!(
        !seen
            .iter()
            .any(|c| matches!(c, Change::NodesOfTypeTruncated { .. })),
        "subscriber must never receive the declarative truncate variant"
    );
}

fn build_fixture_with_subscriber(
    provider: Arc<dyn IndexProvider>,
    subscriber: Arc<dyn ChangeSubscriber>,
) -> SharedGraph {
    let shared = SharedGraph::builder(GraphId::new(2))
        .with_provider(provider)
        .with_change_subscriber(subscriber)
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    {
        let mut m = txn.mutator();
        let l = intern("trunc.sub.L").unwrap();
        let keep = intern("trunc.sub.Keep").unwrap();
        let n0 = m
            .create_node(LabelSet::single(l), PropertyMap::new())
            .unwrap();
        let n1 = m
            .create_node(LabelSet::single(l), PropertyMap::new())
            .unwrap();
        let n2 = m
            .create_node(LabelSet::single(l), PropertyMap::new())
            .unwrap();
        let k = m
            .create_node(LabelSet::single(keep), PropertyMap::new())
            .unwrap();
        let e = intern("trunc.sub.E").unwrap();
        m.create_edge(e, n0, n1, PropertyMap::new()).unwrap();
        m.create_edge(e, n1, n2, PropertyMap::new()).unwrap();
        m.create_edge(e, n2, k, PropertyMap::new()).unwrap();
    }
    txn.commit().unwrap();
    shared
}
