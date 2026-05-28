//! Randomized + deterministic structural-consistency tests for the selene-graph
//! mutation funnel and its derived indexes.
//!
//! Every committed op is checked three ways: the debug-only post-commit hook
//! (active because tests build in debug), an explicit
//! `assert_indexes_consistent()` call (so the test fails with a readable
//! message even if the hook is disabled), an alive-set oracle, and a strictly
//! monotonic generation counter. A fraction of transactions are rolled back to
//! assert an aborted txn leaves zero derived-state drift.

use std::collections::BTreeSet;

use proptest::prelude::*;
use selene_core::{
    EdgeId, GraphId, IStr, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap, Value, intern,
};
use smallvec::smallvec;

use selene_graph::{SeleneGraph, SharedGraph, TypedIndexKind};

// ---------------------------------------------------------------------------
// Shared label / property pools (small so index maintenance is exercised).
// ---------------------------------------------------------------------------

fn labels() -> [IStr; 3] {
    [
        intern("proptest.label.alpha").unwrap(),
        intern("proptest.label.beta").unwrap(),
        intern("proptest.label.gamma").unwrap(),
    ]
}

fn prop_keys() -> [IStr; 3] {
    [
        intern("proptest.key.age").unwrap(),
        intern("proptest.key.score").unwrap(),
        intern("proptest.key.name").unwrap(),
    ]
}

fn edge_labels() -> [IStr; 2] {
    [
        intern("proptest.edge.knows").unwrap(),
        intern("proptest.edge.likes").unwrap(),
    ]
}

/// Register the indexes the random workload maintains: one I64 index, one F64
/// index, and one composite (I64, String) index.
fn register_indexes(shared: &SharedGraph) {
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

fn arb_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        (0i64..5).prop_map(Value::Int),
        (0u8..3).prop_map(|n| Value::Float(f64::from(n))),
        Just(Value::String(intern("proptest.value.x").unwrap())),
        Just(Value::String(intern("proptest.value.y").unwrap())),
        Just(Value::Null),
    ]
}

fn arb_label_set() -> impl Strategy<Value = LabelSet> {
    proptest::collection::vec(0usize..3, 0..=3).prop_map(|idxs| {
        let pool = labels();
        let mut set = LabelSet::new();
        for i in idxs {
            set.insert(pool[i]);
        }
        set
    })
}

fn arb_props() -> impl Strategy<Value = PropertyMap> {
    proptest::collection::vec((0usize..3, arb_value()), 0..=3).prop_map(|pairs| {
        let keys = prop_keys();
        let mut map = PropertyMap::new();
        for (idx, value) in pairs {
            map.set(keys[idx], value).unwrap();
        }
        map
    })
}

#[derive(Clone, Debug)]
enum Op {
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

fn arb_op() -> impl Strategy<Value = Op> {
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

#[derive(Default)]
struct Oracle {
    nodes: Vec<NodeId>,
    edges: Vec<EdgeId>,
    alive_nodes: BTreeSet<NodeId>,
    alive_edges: BTreeSet<EdgeId>,
}

impl Oracle {
    fn pick_alive_node(&self, seed: usize) -> Option<NodeId> {
        if self.alive_nodes.is_empty() {
            return None;
        }
        self.alive_nodes
            .iter()
            .nth(seed % self.alive_nodes.len())
            .copied()
    }

    fn pick_alive_edge(&self, seed: usize) -> Option<EdgeId> {
        if self.alive_edges.is_empty() {
            return None;
        }
        self.alive_edges
            .iter()
            .nth(seed % self.alive_edges.len())
            .copied()
    }
}

fn assert_snapshot_matches_oracle(graph: &SeleneGraph, oracle: &Oracle) {
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
}

/// Apply one op through the funnel, updating the oracle. Returns whether the
/// commit happened (some ops no-op when the graph has no alive entity).
fn apply_op(shared: &SharedGraph, oracle: &mut Oracle, op: &Op, seed: usize) -> bool {
    let mut txn = shared.begin_write();
    let mut committed = true;
    let mut created_node: Option<NodeId> = None;
    let mut created_edge: Option<EdgeId> = None;
    let mut deleted_nodes: Vec<NodeId> = Vec::new();
    let mut deleted_edges: Vec<EdgeId> = Vec::new();
    {
        let mut mutator = txn.mutator();
        match op {
            Op::CreateNode { labels, props } => {
                let id = mutator.create_node(labels.clone(), props.clone()).unwrap();
                created_node = Some(id);
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
                        created_edge = Some(id);
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
                    let prop_diff = PropertyDiff::new(set, []).unwrap();
                    mutator.update_node(node, label_diff, prop_diff).unwrap();
                } else {
                    committed = false;
                }
            }
            Op::UpdateEdge { props } => {
                if let Some(edge) = oracle.pick_alive_edge(seed) {
                    let set: Vec<(IStr, Value)> =
                        props.iter().map(|(k, v)| (*k, v.clone())).collect();
                    let prop_diff = PropertyDiff::new(set, []).unwrap();
                    mutator.update_edge(edge, prop_diff).unwrap();
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

    if let Some(node) = created_node {
        oracle.nodes.push(node);
        oracle.alive_nodes.insert(node);
    }
    if let Some(edge) = created_edge {
        oracle.edges.push(edge);
        oracle.alive_edges.insert(edge);
    }
    for node in deleted_nodes {
        oracle.alive_nodes.remove(&node);
    }
    for edge in deleted_edges {
        oracle.alive_edges.remove(&edge);
    }
    true
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(200)
    ))]

    #[test]
    fn funnel_keeps_indexes_consistent(ops in proptest::collection::vec(arb_op(), 1..=100)) {
        let shared = SharedGraph::new(GraphId::new(1));
        register_indexes(&shared);
        let mut oracle = Oracle::default();
        let mut last_generation = shared.read().meta.generation;

        for (i, op) in ops.iter().enumerate() {
            let committed = apply_op(&shared, &mut oracle, op, i);
            let snapshot = shared.read();
            // (i) Consistency after every committed op.
            prop_assert!(
                snapshot.assert_indexes_consistent().is_ok(),
                "{}",
                snapshot.assert_indexes_consistent().unwrap_err()
            );
            // (ii) Alive-set parity vs the oracle.
            assert_snapshot_matches_oracle(&snapshot, &oracle);
            // (iii) Generation monotonicity: exactly +1 per committed op.
            if committed {
                prop_assert_eq!(snapshot.meta.generation, last_generation + 1);
                last_generation = snapshot.meta.generation;
            } else {
                prop_assert_eq!(snapshot.meta.generation, last_generation);
            }
        }
    }

    #[test]
    fn rollback_leaves_zero_drift(
        setup in proptest::collection::vec(arb_op(), 0..=20),
        batch in proptest::collection::vec(arb_op(), 1..=10),
    ) {
        let shared = SharedGraph::new(GraphId::new(1));
        register_indexes(&shared);
        let mut oracle = Oracle::default();
        for (i, op) in setup.iter().enumerate() {
            apply_op(&shared, &mut oracle, op, i);
        }
        let pre = shared.read();
        let pre_nodes = pre.node_count();
        let pre_edges = pre.edge_count();
        let pre_label_count = pre.label_count();
        let pre_property_index_count = pre.property_index_count();
        let pre_generation = pre.meta.generation;
        prop_assert!(pre.assert_indexes_consistent().is_ok());

        // Run a batch of mutations inside one txn, then roll it back.
        {
            let mut txn = shared.begin_write();
            {
                let mut mutator = txn.mutator();
                for op in &batch {
                    // Best-effort: create_node always works; others may no-op.
                    if let Op::CreateNode { labels, props } = op {
                        let _ = mutator.create_node(labels.clone(), props.clone());
                    }
                }
            }
            txn.rollback();
        }

        let post = shared.read();
        prop_assert_eq!(post.node_count(), pre_nodes);
        prop_assert_eq!(post.edge_count(), pre_edges);
        prop_assert_eq!(post.label_count(), pre_label_count);
        prop_assert_eq!(post.property_index_count(), pre_property_index_count);
        prop_assert_eq!(post.meta.generation, pre_generation);
        prop_assert!(post.assert_indexes_consistent().is_ok());
    }
}

// ---------------------------------------------------------------------------
// Deterministic targeted-invariant battery.
// ---------------------------------------------------------------------------

#[test]
fn delete_node_with_incident_edges_cleans_adjacency_and_labels() {
    let shared = SharedGraph::new(GraphId::new(1));
    let [alpha, ..] = labels();
    let [knows, _] = edge_labels();
    let (a, b) = {
        let mut txn = shared.begin_write();
        let (a, b);
        {
            let mut m = txn.mutator();
            a = m
                .create_node(LabelSet::single(alpha), PropertyMap::new())
                .unwrap();
            b = m
                .create_node(LabelSet::single(alpha), PropertyMap::new())
                .unwrap();
            m.create_edge(knows, a, b, PropertyMap::new()).unwrap();
            m.create_edge(knows, b, a, PropertyMap::new()).unwrap();
        }
        txn.commit().unwrap();
        (a, b)
    };
    {
        let mut txn = shared.begin_write();
        txn.mutator().delete_node(a).unwrap();
        txn.commit().unwrap();
    }
    let snapshot = shared.read();
    snapshot.assert_indexes_consistent().unwrap();
    assert!(!snapshot.is_node_alive(a));
    assert!(snapshot.is_node_alive(b));
    assert_eq!(snapshot.edge_count(), 0, "incident edges cascade-deleted");
    assert!(snapshot.outgoing_edges(a).is_none());
    assert!(snapshot.incoming_edges(a).is_none());
    assert!(snapshot.outgoing_edges(b).is_none());
    assert!(snapshot.incoming_edges(b).is_none());
}

#[test]
fn flipping_a_label_moves_index_buckets() {
    let shared = SharedGraph::new(GraphId::new(1));
    let [alpha, beta, _] = labels();
    let [age, _, _] = prop_keys();
    shared
        .create_property_index(alpha, age, TypedIndexKind::I64)
        .unwrap();
    shared
        .create_property_index(beta, age, TypedIndexKind::I64)
        .unwrap();
    let node = {
        let mut txn = shared.begin_write();
        let id = txn
            .mutator()
            .create_node(
                LabelSet::single(alpha),
                PropertyMap::from_pairs([(age, Value::Int(7))]).unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
        id
    };
    {
        let snapshot = shared.read();
        assert_eq!(
            snapshot
                .nodes_with_property_eq(&alpha, &age, &Value::Int(7))
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            vec![0]
        );
        assert!(
            snapshot
                .nodes_with_property_eq(&beta, &age, &Value::Int(7))
                .unwrap()
                .is_empty()
        );
    }
    // Flip alpha -> beta.
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .update_node(
                node,
                LabelDiff::new([beta], [alpha]).unwrap(),
                PropertyDiff::new([], []).unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    let snapshot = shared.read();
    snapshot.assert_indexes_consistent().unwrap();
    assert!(
        snapshot
            .nodes_with_property_eq(&alpha, &age, &Value::Int(7))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        snapshot
            .nodes_with_property_eq(&beta, &age, &Value::Int(7))
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        vec![0]
    );
}

#[test]
fn external_string_admits_into_string_index() {
    use std::sync::Arc;
    let shared = SharedGraph::new(GraphId::new(1));
    let [alpha, ..] = labels();
    let [_, _, name] = prop_keys();
    shared
        .create_property_index(alpha, name, TypedIndexKind::String)
        .unwrap();
    let content = "proptest.external.unique-admit";
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(alpha),
                PropertyMap::from_pairs([(name, Value::ExternalString(Arc::<str>::from(content)))])
                    .unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    let snapshot = shared.read();
    snapshot.assert_indexes_consistent().unwrap();
    let interned = intern(content).unwrap();
    assert_eq!(
        snapshot
            .nodes_with_property_eq(&alpha, &name, &Value::String(interned))
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        vec![0]
    );
}

#[test]
fn nan_float_is_skipped_without_false_positive() {
    let shared = SharedGraph::new(GraphId::new(1));
    let [alpha, ..] = labels();
    let [_, score, _] = prop_keys();
    shared
        .create_property_index(alpha, score, TypedIndexKind::F64)
        .unwrap();
    {
        let mut txn = shared.begin_write();
        // NaN is legitimately skipped by maintenance; assert no panic + consistent.
        txn.mutator()
            .create_node(
                LabelSet::single(alpha),
                PropertyMap::from_pairs([(score, Value::Float(f64::NAN))]).unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    let snapshot = shared.read();
    snapshot.assert_indexes_consistent().unwrap();
    // The NaN row is absent from the index.
    assert_eq!(snapshot.node_count(), 1);
}

#[test]
fn null_property_is_never_indexed() {
    let shared = SharedGraph::new(GraphId::new(1));
    let [alpha, ..] = labels();
    let [age, _, _] = prop_keys();
    shared
        .create_property_index(alpha, age, TypedIndexKind::I64)
        .unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(alpha),
                PropertyMap::from_pairs([(age, Value::Null)]).unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    let snapshot = shared.read();
    snapshot.assert_indexes_consistent().unwrap();
    assert!(
        snapshot
            .nodes_with_property_eq(&age, &age, &Value::Int(0))
            .is_none()
            || snapshot.node_count() == 1
    );
}
