use std::sync::Arc;

use super::*;

#[test]
fn read_within_tx_sees_own_writes() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let id = empty_node(&mut mutator);
    assert!(mutator.read().is_node_alive(id));
}

#[test]
fn read_within_tx_sees_label_index_updates() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let label = db_string("node.index.tx-read").unwrap();
    let mut mutator = txn.mutator();
    let id = mutator
        .create_node(LabelSet::single(label.clone()), PropertyMap::new())
        .expect("create_node ok");
    let row = mutator
        .read()
        .row_for_node_id(id)
        .expect("created node is mapped")
        .get();
    assert!(
        mutator
            .read()
            .nodes_with_label(&label)
            .unwrap()
            .contains(row)
    );
}

#[test]
fn multi_step_tx_emits_changes_in_order() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let id = {
        let mut mutator = txn.mutator();
        let id = empty_node(&mut mutator);
        mutator
            .update_node(
                id,
                LabelDiff::new([db_string("node.updated").unwrap()], []).unwrap(),
                PropertyDiff::new([], []).unwrap(),
            )
            .unwrap();
        mutator.delete_node(id).unwrap();
        id
    };
    let outcome = txn.commit().unwrap();
    assert!(matches!(outcome.changes[0], Change::NodeCreated { .. }));
    assert!(matches!(outcome.changes[1], Change::NodeUpdated { .. }));
    assert_eq!(outcome.changes[2], Change::NodeDeleted { id });
}

#[test]
fn schema_change_emits_change_passthrough() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator.schema_change(
            GraphId::new(1),
            SchemaChange::GraphDropped {
                id: GraphId::new(2),
            },
        );
    }
    let outcome = txn.commit().unwrap();
    assert!(matches!(outcome.changes[0], Change::SchemaChanged { .. }));
}

#[test]
#[cfg(not(miri))]
fn four_writer_stress_no_double_allocation() {
    let shared = Arc::new(SharedGraph::new(GraphId::new(1)));
    let nodes_per_thread = 64;
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let shared = Arc::clone(&shared);
            scope.spawn(move || {
                let mut txn = shared.begin_write();
                {
                    let mut mutator = txn.mutator();
                    for _ in 0..nodes_per_thread {
                        mutator
                            .create_node(LabelSet::new(), PropertyMap::new())
                            .expect("create_node ok");
                    }
                }
                txn.commit().unwrap();
            });
        }
    });
    let snapshot = shared.read();
    assert_eq!(snapshot.node_count(), 4 * nodes_per_thread);
    assert_eq!(
        snapshot.meta.next_node_id,
        (4 * nodes_per_thread + 1) as u64
    );
}

#[test]
fn value_type_import_smoke_keeps_schema_deferred() {
    let value_type = ValueType::predefined(PredefinedValueType::String);
    assert_eq!(value_type.predefined, Some(PredefinedValueType::String));
}
