//! Shared randomized-funnel test harness for the selene-graph integration
//! tests.
//!
//! Lives under the special `tests/<name>/mod.rs` layout so cargo treats it as a
//! plain module (not its own test binary). It is `#[path]`-free-included via
//! `mod funnel_harness;` by both `property_tests.rs` (in-memory consistency) and
//! `recovery_property.rs` (durability round-trip), which share the [`Oracle`],
//! the op generators, and [`apply_op`] — the funnel driver that keeps the oracle
//! in lock-step with what the mutation funnel actually stored (including the
//! exact label/property CONTENT per alive id, the half the structural net cannot
//! see).

#![allow(dead_code)] // Each consumer uses a subset of the shared surface.

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use selene_core::{
    EdgeId, IStr, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap, Value, intern,
};
use smallvec::smallvec;

use selene_graph::{SeleneGraph, SharedGraph, TypedIndexKind};

// ---------------------------------------------------------------------------
// Shared label / property pools (small so index maintenance is exercised).
// ---------------------------------------------------------------------------

pub fn labels() -> [IStr; 3] {
    [
        intern("proptest.label.alpha").unwrap(),
        intern("proptest.label.beta").unwrap(),
        intern("proptest.label.gamma").unwrap(),
    ]
}

pub fn prop_keys() -> [IStr; 3] {
    [
        intern("proptest.key.age").unwrap(),
        intern("proptest.key.score").unwrap(),
        intern("proptest.key.name").unwrap(),
    ]
}

pub fn edge_labels() -> [IStr; 2] {
    [
        intern("proptest.edge.knows").unwrap(),
        intern("proptest.edge.likes").unwrap(),
    ]
}

/// Register the indexes the random workload maintains: one I64 index, one F64
/// index, and one composite (I64, String) index.
pub fn register_indexes(shared: &SharedGraph) {
    let [alpha, ..] = labels();
    let [age, score, name] = prop_keys();
    shared
        .create_property_index(alpha, age, TypedIndexKind::I64)
        .unwrap();
    shared
        .create_property_index(alpha, score, TypedIndexKind::F64)
        .unwrap();
    let props: smallvec::SmallVec<[IStr; 4]> = smallvec![age, name];
    let kinds: smallvec::SmallVec<[TypedIndexKind; 4]> =
        smallvec![TypedIndexKind::I64, TypedIndexKind::String];
    let mut txn = shared.begin_write();
    txn.mutator()
        .create_composite_property_index_named(alpha, props, kinds, None)
        .expect("composite index registration");
    txn.commit().unwrap();
}

// ---------------------------------------------------------------------------
// Random value generators.
// ---------------------------------------------------------------------------

pub fn arb_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        (0i64..5).prop_map(Value::Int),
        (0u8..3).prop_map(|n| Value::Float(f64::from(n))),
        Just(Value::String(intern("proptest.value.x").unwrap())),
        Just(Value::String(intern("proptest.value.y").unwrap())),
        Just(Value::Null),
    ]
}

pub fn arb_label_set() -> impl Strategy<Value = LabelSet> {
    proptest::collection::vec(0usize..3, 0..=3).prop_map(|idxs| {
        let pool = labels();
        let mut set = LabelSet::new();
        for i in idxs {
            set.insert(pool[i]);
        }
        set
    })
}

pub fn arb_props() -> impl Strategy<Value = PropertyMap> {
    proptest::collection::vec((0usize..3, arb_value()), 0..=3).prop_map(|pairs| {
        let keys = prop_keys();
        let mut map = PropertyMap::new();
        for (idx, value) in pairs {
            map.set(keys[idx], value).unwrap();
        }
        map
    })
}

/// One random mutation against the funnel.
#[derive(Clone, Debug)]
pub enum Op {
    CreateNode {
        labels: LabelSet,
        props: PropertyMap,
    },
    CreateEdge {
        label_idx: usize,
    },
    UpdateNode {
        props: PropertyMap,
        flip_label: usize,
    },
    UpdateEdge {
        props: PropertyMap,
    },
    DeleteNode,
    DeleteEdge,
}

pub fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (arb_label_set(), arb_props()).prop_map(|(labels, props)| Op::CreateNode { labels, props }),
        2 => (0usize..2).prop_map(|label_idx| Op::CreateEdge { label_idx }),
        2 => (arb_props(), 0usize..3).prop_map(|(props, flip_label)| Op::UpdateNode { props, flip_label }),
        1 => arb_props().prop_map(|props| Op::UpdateEdge { props }),
        1 => Just(Op::DeleteNode),
        1 => Just(Op::DeleteEdge),
    ]
}

// ---------------------------------------------------------------------------
// Oracle.
// ---------------------------------------------------------------------------

/// The expected post-commit state, maintained in lock-step with `apply_op`.
#[derive(Default)]
pub struct Oracle {
    pub nodes: Vec<NodeId>,
    pub edges: Vec<EdgeId>,
    pub alive_nodes: BTreeSet<NodeId>,
    pub alive_edges: BTreeSet<EdgeId>,
    /// Expected label set per alive node — the content shadow the structural
    /// consistency net cannot provide (it re-derives indexes FROM the columns,
    /// so a value written to the wrong row passes the net but not this map).
    pub node_labels: BTreeMap<NodeId, LabelSet>,
    /// Expected property map per alive node.
    pub node_props: BTreeMap<NodeId, PropertyMap>,
    /// Expected property map per alive edge.
    pub edge_props: BTreeMap<EdgeId, PropertyMap>,
}

impl Oracle {
    pub fn pick_alive_node(&self, seed: usize) -> Option<NodeId> {
        if self.alive_nodes.is_empty() {
            return None;
        }
        self.alive_nodes
            .iter()
            .nth(seed % self.alive_nodes.len())
            .copied()
    }

    pub fn pick_alive_edge(&self, seed: usize) -> Option<EdgeId> {
        if self.alive_edges.is_empty() {
            return None;
        }
        self.alive_edges
            .iter()
            .nth(seed % self.alive_edges.len())
            .copied()
    }
}

/// Assert alive-set + count parity, then exact content (see
/// [`assert_content_matches_oracle`]).
pub fn assert_snapshot_matches_oracle(graph: &SeleneGraph, oracle: &Oracle) {
    assert_eq!(
        graph.node_count(),
        oracle.alive_nodes.len(),
        "node_count drift vs oracle"
    );
    assert_eq!(
        graph.edge_count(),
        oracle.alive_edges.len(),
        "edge_count drift vs oracle"
    );
    for node in &oracle.nodes {
        assert_eq!(
            graph.is_node_alive(*node),
            oracle.alive_nodes.contains(node),
            "node {node} liveness drift"
        );
    }
    for edge in &oracle.edges {
        assert_eq!(
            graph.is_edge_alive(*edge),
            oracle.alive_edges.contains(edge),
            "edge {edge} liveness drift"
        );
    }
    assert_content_matches_oracle(graph, oracle);
}

/// Assert that every alive id resolves to the EXACT label set / property map the
/// oracle recorded. This is the content half that liveness+count parity cannot
/// see: the structural consistency net re-derives the label/property indexes
/// FROM the stored columns, so a value written to the wrong row (the D22 row↔id
/// hazard) is self-consistent under the net yet wrong here. A regression that
/// transposed two rows' properties, or resolved an id to the wrong row, would
/// fail this and pass `assert_indexes_consistent`.
pub fn assert_content_matches_oracle(graph: &SeleneGraph, oracle: &Oracle) {
    for node in &oracle.alive_nodes {
        let expected_labels = oracle
            .node_labels
            .get(node)
            .expect("alive node has oracle labels");
        assert_eq!(
            graph.node_labels(*node),
            Some(expected_labels),
            "node {node} label-set content drift"
        );
        let expected_props = oracle
            .node_props
            .get(node)
            .expect("alive node has oracle props");
        assert_eq!(
            graph.node_properties(*node),
            Some(expected_props),
            "node {node} property content drift"
        );
    }
    for edge in &oracle.alive_edges {
        let expected_props = oracle
            .edge_props
            .get(edge)
            .expect("alive edge has oracle props");
        assert_eq!(
            graph.edge_properties(*edge),
            Some(expected_props),
            "edge {edge} property content drift"
        );
    }
}

/// Apply one op through the funnel, updating the oracle. Returns whether the
/// commit happened (some ops no-op when the graph has no alive entity).
pub fn apply_op(shared: &SharedGraph, oracle: &mut Oracle, op: &Op, seed: usize) -> bool {
    let mut txn = shared.begin_write();
    let mut committed = true;
    // Content transitions captured under the txn, replayed into the oracle's
    // content maps post-commit so the maps mirror exactly what the funnel stored.
    let mut created_node: Option<(NodeId, LabelSet, PropertyMap)> = None;
    let mut created_edge: Option<(EdgeId, PropertyMap)> = None;
    let mut node_content_update: Option<(NodeId, LabelSet, PropertyMap)> = None;
    let mut edge_content_update: Option<(EdgeId, PropertyMap)> = None;
    let mut deleted_nodes: Vec<NodeId> = Vec::new();
    let mut deleted_edges: Vec<EdgeId> = Vec::new();
    {
        let mut mutator = txn.mutator();
        match op {
            Op::CreateNode { labels, props } => {
                let id = mutator.create_node(labels.clone(), props.clone()).unwrap();
                // Open graph (no bound type) → create stores labels/props verbatim
                // with no default fill, so the oracle mirrors them exactly.
                created_node = Some((id, labels.clone(), props.clone()));
            }
            Op::CreateEdge { label_idx } => {
                let source = oracle.pick_alive_node(seed);
                let target = oracle.pick_alive_node(seed.wrapping_add(7));
                match (source, target) {
                    (Some(source), Some(target)) => {
                        let label = edge_labels()[*label_idx];
                        let id = mutator
                            .create_edge(label, source, target, PropertyMap::new())
                            .unwrap();
                        created_edge = Some((id, PropertyMap::new()));
                    }
                    _ => committed = false,
                }
            }
            Op::UpdateNode { props, flip_label } => {
                if let Some(node) = oracle.pick_alive_node(seed) {
                    let label = labels()[*flip_label];
                    let has = shared
                        .read()
                        .node_labels(node)
                        .is_some_and(|set| set.contains(&label));
                    let label_diff = if has {
                        LabelDiff::new([], [label]).unwrap()
                    } else {
                        LabelDiff::new([label], []).unwrap()
                    };
                    let set: Vec<(IStr, Value)> =
                        props.iter().map(|(k, v)| (*k, v.clone())).collect();
                    let prop_diff = PropertyDiff::new(set.clone(), []).unwrap();
                    mutator.update_node(node, label_diff, prop_diff).unwrap();
                    // Mirror the funnel's diff merge (apply_property_diff +
                    // label add/remove) against the oracle's prior content.
                    let mut new_labels = oracle.node_labels[&node].clone();
                    if has {
                        new_labels.remove(&label);
                    } else {
                        new_labels.insert(label);
                    }
                    let mut new_props = oracle.node_props[&node].clone();
                    for (k, v) in set {
                        new_props.set(k, v).unwrap();
                    }
                    node_content_update = Some((node, new_labels, new_props));
                } else {
                    committed = false;
                }
            }
            Op::UpdateEdge { props } => {
                if let Some(edge) = oracle.pick_alive_edge(seed) {
                    let set: Vec<(IStr, Value)> =
                        props.iter().map(|(k, v)| (*k, v.clone())).collect();
                    let prop_diff = PropertyDiff::new(set.clone(), []).unwrap();
                    mutator.update_edge(edge, prop_diff).unwrap();
                    let mut new_props = oracle.edge_props[&edge].clone();
                    for (k, v) in set {
                        new_props.set(k, v).unwrap();
                    }
                    edge_content_update = Some((edge, new_props));
                } else {
                    committed = false;
                }
            }
            Op::DeleteNode => {
                if let Some(node) = oracle.pick_alive_node(seed) {
                    // Cascade also deletes incident edges; capture them.
                    let snapshot = shared.read();
                    let mut incident = BTreeSet::new();
                    if let Some(out) = snapshot.outgoing_edges(node) {
                        incident.extend(out.iter().map(|e| e.edge_id));
                    }
                    if let Some(inc) = snapshot.incoming_edges(node) {
                        incident.extend(inc.iter().map(|e| e.edge_id));
                    }
                    mutator.delete_node(node).unwrap();
                    deleted_nodes.push(node);
                    deleted_edges.extend(incident);
                } else {
                    committed = false;
                }
            }
            Op::DeleteEdge => {
                if let Some(edge) = oracle.pick_alive_edge(seed) {
                    mutator.delete_edge(edge).unwrap();
                    deleted_edges.push(edge);
                } else {
                    committed = false;
                }
            }
        }
    }

    if !committed {
        txn.rollback();
        return false;
    }
    txn.commit().unwrap();

    if let Some((node, labels, props)) = created_node {
        oracle.nodes.push(node);
        oracle.alive_nodes.insert(node);
        oracle.node_labels.insert(node, labels);
        oracle.node_props.insert(node, props);
    }
    if let Some((edge, props)) = created_edge {
        oracle.edges.push(edge);
        oracle.alive_edges.insert(edge);
        oracle.edge_props.insert(edge, props);
    }
    if let Some((node, labels, props)) = node_content_update {
        oracle.node_labels.insert(node, labels);
        oracle.node_props.insert(node, props);
    }
    if let Some((edge, props)) = edge_content_update {
        oracle.edge_props.insert(edge, props);
    }
    for node in deleted_nodes {
        oracle.alive_nodes.remove(&node);
        oracle.node_labels.remove(&node);
        oracle.node_props.remove(&node);
    }
    for edge in deleted_edges {
        oracle.alive_edges.remove(&edge);
        oracle.edge_props.remove(&edge);
    }
    true
}
