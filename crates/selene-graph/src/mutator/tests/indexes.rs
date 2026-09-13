use super::*;

#[test]
fn create_node_returns_id_and_emits_change() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let id = {
        let mut mutator = txn.mutator();
        mutator
            .create_node(LabelSet::new(), PropertyMap::new())
            .expect("create_node ok")
    };
    let outcome = txn.commit().unwrap();
    assert_eq!(id, NodeId::new(1));
    assert!(matches!(outcome.changes[0], Change::NodeCreated { id, .. } if id == NodeId::new(1)));
}

#[test]
fn create_node_with_two_labels_indexes_both() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (a, b, id) = {
        let mut mutator = txn.mutator();
        let a = db_string("node.index.a").unwrap();
        let b = db_string("node.index.b").unwrap();
        let id = mutator
            .create_node(
                LabelSet::from_iter([a.clone(), b.clone()]),
                PropertyMap::new(),
            )
            .expect("create_node ok");
        assert!(mutator.read().nodes_with_label(&a).unwrap().contains(0));
        assert!(mutator.read().nodes_with_label(&b).unwrap().contains(0));
        (a, b, id)
    };
    txn.commit().unwrap();
    let snapshot = shared.read();
    assert_eq!(id, NodeId::new(1));
    assert!(snapshot.nodes_with_label(&a).unwrap().contains(0));
    assert!(snapshot.nodes_with_label(&b).unwrap().contains(0));
}

#[test]
fn update_node_label_diff_updates_indexes_atomically() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (old, new, id) = {
        let mut mutator = txn.mutator();
        let old = db_string("node.index.old").unwrap();
        let new = db_string("node.index.new").unwrap();
        let id = mutator
            .create_node(LabelSet::single(old.clone()), PropertyMap::new())
            .expect("create_node ok");
        mutator
            .update_node(
                id,
                LabelDiff::new([new.clone()], [old.clone()]).unwrap(),
                PropertyDiff::new([], []).unwrap(),
            )
            .unwrap();
        assert!(mutator.read().nodes_with_label(&old).is_none());
        assert!(mutator.read().nodes_with_label(&new).unwrap().contains(0));
        (old, new, id)
    };
    txn.commit().unwrap();
    let snapshot = shared.read();
    assert_eq!(id, NodeId::new(1));
    assert!(snapshot.nodes_with_label(&old).is_none());
    assert!(snapshot.nodes_with_label(&new).unwrap().contains(0));
}

#[test]
fn delete_node_removes_from_all_label_indexes() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (a, b) = {
        let mut mutator = txn.mutator();
        let a = db_string("node.index.delete.a").unwrap();
        let b = db_string("node.index.delete.b").unwrap();
        let id = mutator
            .create_node(
                LabelSet::from_iter([a.clone(), b.clone()]),
                PropertyMap::new(),
            )
            .expect("create_node ok");
        mutator.delete_node(id).unwrap();
        assert!(mutator.read().nodes_with_label(&a).is_none());
        assert!(mutator.read().nodes_with_label(&b).is_none());
        (a, b)
    };
    txn.commit().unwrap();
    assert!(shared.read().nodes_with_label(&a).is_none());
    assert!(shared.read().nodes_with_label(&b).is_none());
}

#[test]
fn update_node_with_unknown_id_fails() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let err = mutator
        .update_node(
            NodeId::new(1),
            LabelDiff::new([], []).unwrap(),
            PropertyDiff::new([], []).unwrap(),
        )
        .unwrap_err();
    assert!(matches!(err, GraphError::NodeNotFound { .. }));
}

#[test]
fn delete_node_cascade_clears_edge_label_index() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let label = db_string("edge.index.cascade").unwrap();
    {
        let mut mutator = txn.mutator();
        let a = empty_node(&mut mutator);
        let b = empty_node(&mut mutator);
        mutator
            .create_edge(label.clone(), a, b, PropertyMap::new())
            .expect("create_edge ok");
        assert!(mutator.read().edges_with_label(&label).unwrap().contains(0));
        mutator.delete_node(a).unwrap();
        assert!(mutator.read().edges_with_label(&label).is_none());
    }
    txn.commit().unwrap();
    assert!(shared.read().edges_with_label(&label).is_none());
}

#[test]
fn index_entry_dropped_when_bitmap_empties() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let label = db_string("node.index.drop-empty").unwrap();
    {
        let mut mutator = txn.mutator();
        let id = mutator
            .create_node(LabelSet::single(label.clone()), PropertyMap::new())
            .expect("create_node ok");
        assert_eq!(mutator.read().label_count(), 1);
        mutator.delete_node(id).unwrap();
        assert_eq!(mutator.read().label_count(), 0);
        assert!(mutator.read().idx_label.get(&label).is_none());
    }
    txn.commit().unwrap();
    assert_eq!(shared.read().label_count(), 0);
}

#[test]
fn update_node_with_no_label_changes_does_not_touch_indexes() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let label = db_string("node.index.props-only").unwrap();
    let prop = db_string("node.index.prop").unwrap();
    {
        let mut mutator = txn.mutator();
        let id = mutator
            .create_node(LabelSet::single(label), PropertyMap::new())
            .expect("create_node ok");
        let before = mutator.read().idx_label.clone();
        mutator
            .update_node(
                id,
                LabelDiff::new([], []).unwrap(),
                PropertyDiff::new([(prop, Value::Int(1))], []).unwrap(),
            )
            .unwrap();
        assert_eq!(mutator.read().idx_label, before);
    }
    txn.commit().unwrap();
}
