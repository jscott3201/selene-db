use std::collections::BTreeSet;

use selene_core::{Change, EdgeId, GraphId, LabelSet, NodeId, PropertyMap, db_string};

use super::*;
use crate::SharedGraph;

fn empty_node(mutator: &mut Mutator<'_, '_>) -> NodeId {
    mutator
        .create_node(LabelSet::new(), PropertyMap::new())
        .expect("create_node ok")
}

fn edge(mutator: &mut Mutator<'_, '_>, source: NodeId, target: NodeId) -> EdgeId {
    mutator
        .create_edge(
            db_string("delete.set.edge").unwrap(),
            source,
            target,
            PropertyMap::new(),
        )
        .expect("create_edge ok")
}

fn set<T: Ord>(items: impl IntoIterator<Item = T>) -> BTreeSet<T> {
    items.into_iter().collect()
}

#[test]
fn delete_elements_dedupes_nodes_and_edges() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (a, b, edge_id) = {
        let mut mutator = txn.mutator();
        let a = empty_node(&mut mutator);
        let b = empty_node(&mut mutator);
        let edge_id = edge(&mut mutator, a, b);

        mutator
            .delete_elements(set([a, a]), set([edge_id, edge_id]))
            .expect("delete set applies");
        (a, b, edge_id)
    };

    let outcome = txn.commit().expect("commit ok");
    assert_eq!(
        &outcome.changes[outcome.changes.len() - 2..],
        [
            Change::NodeDeleted { id: a },
            Change::EdgeDeleted { id: edge_id },
        ]
    );
    let snapshot = shared.read();
    assert!(!snapshot.is_node_alive(a));
    assert!(snapshot.is_node_alive(b));
    assert!(!snapshot.is_edge_alive(edge_id));
}

#[test]
fn delete_elements_keeps_explicit_nonincident_edges() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (a, b, c, d, incident, explicit) = {
        let mut mutator = txn.mutator();
        let a = empty_node(&mut mutator);
        let b = empty_node(&mut mutator);
        let c = empty_node(&mut mutator);
        let d = empty_node(&mut mutator);
        let incident = edge(&mut mutator, a, b);
        let explicit = edge(&mut mutator, c, d);

        mutator
            .delete_elements(set([a]), set([incident, explicit, explicit]))
            .expect("delete set applies");
        (a, b, c, d, incident, explicit)
    };

    txn.commit().expect("commit ok");
    let snapshot = shared.read();
    assert!(!snapshot.is_node_alive(a));
    assert!(snapshot.is_node_alive(b));
    assert!(snapshot.is_node_alive(c));
    assert!(snapshot.is_node_alive(d));
    assert!(!snapshot.is_edge_alive(incident));
    assert!(!snapshot.is_edge_alive(explicit));
}

#[test]
fn delete_elements_skips_already_invalidated_ids() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (a, b, edge_id) = {
        let mut mutator = txn.mutator();
        let a = empty_node(&mut mutator);
        let b = empty_node(&mut mutator);
        let edge_id = edge(&mut mutator, a, b);

        mutator
            .delete_elements(set([a]), set([]))
            .expect("first delete");
        mutator
            .delete_elements(set([a]), set([edge_id]))
            .expect("second delete is a no-op");
        (a, b, edge_id)
    };

    let outcome = txn.commit().expect("commit ok");
    assert_eq!(
        outcome
            .changes
            .iter()
            .filter(|change| {
                matches!(
                    change,
                    Change::NodeDeleted { .. } | Change::EdgeDeleted { .. }
                )
            })
            .count(),
        2
    );
    let snapshot = shared.read();
    assert!(!snapshot.is_node_alive(a));
    assert!(snapshot.is_node_alive(b));
    assert!(!snapshot.is_edge_alive(edge_id));
}

#[test]
fn delete_elements_rejects_missing_node_ids() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let err = mutator
        .delete_elements(set([NodeId::new(99)]), set([]))
        .expect_err("missing node id is rejected");
    assert!(matches!(
        err,
        GraphError::NodeNotFound { id } if id == NodeId::new(99)
    ));
}

#[test]
fn delete_elements_rejects_missing_edge_ids() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let err = mutator
        .delete_elements(set([]), set([EdgeId::new(99)]))
        .expect_err("missing edge id is rejected");
    assert!(matches!(
        err,
        GraphError::EdgeNotFound { id } if id == EdgeId::new(99)
    ));
}
