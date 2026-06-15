use super::*;

#[test]
fn create_edge_with_invalid_source_fails() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let target = empty_node(&mut mutator);
    let err = mutator
        .create_edge(
            db_string("edge.invalid.source").unwrap(),
            NodeId::new(99),
            target,
            PropertyMap::new(),
        )
        .unwrap_err();
    assert!(matches!(err, GraphError::NodeNotFound { id } if id == NodeId::new(99)));
}

#[test]
fn create_edge_with_invalid_target_fails() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let source = empty_node(&mut mutator);
    let err = mutator
        .create_edge(
            db_string("edge.invalid.target").unwrap(),
            source,
            NodeId::new(99),
            PropertyMap::new(),
        )
        .unwrap_err();
    assert!(matches!(err, GraphError::NodeNotFound { id } if id == NodeId::new(99)));
}

#[test]
fn delete_node_cascades_to_incident_edges() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (a, b, edge) = {
        let mut mutator = txn.mutator();
        let a = empty_node(&mut mutator);
        let b = empty_node(&mut mutator);
        let edge = mutator
            .create_edge(db_string("edge.cascade").unwrap(), a, b, PropertyMap::new())
            .unwrap();
        mutator.delete_node(a).unwrap();
        (a, b, edge)
    };
    txn.commit().unwrap();
    let snapshot = shared.read();
    assert!(!snapshot.is_node_alive(a));
    assert!(snapshot.is_node_alive(b));
    assert!(!snapshot.is_edge_alive(edge));
    assert!(snapshot.incoming_edges(b).is_none());
}

#[test]
fn delete_edge_updates_both_adjacencies() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (a, b, edge) = {
        let mut mutator = txn.mutator();
        let a = empty_node(&mut mutator);
        let b = empty_node(&mut mutator);
        let edge = mutator
            .create_edge(db_string("edge.delete").unwrap(), a, b, PropertyMap::new())
            .unwrap();
        mutator.delete_edge(edge).unwrap();
        (a, b, edge)
    };
    txn.commit().unwrap();
    let snapshot = shared.read();
    assert!(!snapshot.is_edge_alive(edge));
    assert!(snapshot.outgoing_edges(a).is_none());
    assert!(snapshot.incoming_edges(b).is_none());
}

#[test]
fn update_edge_updates_properties() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (edge, prop) = {
        let mut mutator = txn.mutator();
        let a = empty_node(&mut mutator);
        let b = empty_node(&mut mutator);
        let edge = mutator
            .create_edge(db_string("edge.update").unwrap(), a, b, PropertyMap::new())
            .unwrap();
        let prop = db_string("edge.prop").unwrap();
        mutator
            .update_edge(
                edge,
                PropertyDiff::new([(prop.clone(), Value::String(prop.clone()))], []).unwrap(),
            )
            .unwrap();
        (edge, prop)
    };
    txn.commit().unwrap();
    assert_eq!(
        shared.read().edge_properties(edge).unwrap().get(&prop),
        Some(&Value::String(prop))
    );
}
