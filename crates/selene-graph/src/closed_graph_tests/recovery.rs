use super::*;

#[test]
fn recover_round_trips_bound_graph_type_and_rearms_validator() {
    let dir = temp_dir("roundtrip");
    let graph_type = person_graph_type();
    let shared = SharedGraph::builder(GraphId::new(5))
        .bound_to(graph_type.clone())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let alice = mutator
            .create_node(
                LabelSet::single(db_string("Person")),
                prop("name", Value::String(db_string("Alice"))),
            )
            .unwrap();
        let bob = mutator
            .create_node(
                LabelSet::single(db_string("Person")),
                prop("name", Value::String(db_string("Bob"))),
            )
            .unwrap();
        mutator
            .create_edge(
                db_string("KNOWS"),
                alice,
                bob,
                prop("since", Value::Int(2026)),
            )
            .unwrap();
    }
    txn.commit().unwrap();
    let sequence = shared.read().meta.generation;
    write_snapshot(&dir, &shared, sequence);
    append_wal(
        &dir,
        sequence,
        &[Change::NodeCreated {
            id: NodeId::new(3),
            labels: LabelSet::single(db_string("Person")),
            properties: prop("name", Value::String(db_string("Carol"))),
        }],
    );

    let recovered = SharedGraph::recover_closed(&dir, GraphId::new(5), graph_type.clone()).unwrap();
    assert!(recovered.is_closed());
    assert_eq!(recovered.graph_type().as_deref(), Some(&graph_type));
    assert!(recovered.read().is_node_alive(NodeId::new(3)));

    let mut txn = recovered.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_edge(
                db_string("KNOWS"),
                NodeId::new(1),
                NodeId::new(2),
                prop("since", Value::String(db_string("bad"))),
            )
            .unwrap();
    }
    assert!(matches!(
        txn.commit().unwrap_err(),
        GraphError::TypeViolation(TypeViolation::PropertyTypeMismatch {
            entity_id,
            property,
            expected: PropertyValueType::Int,
            observed: "String",
        }) if entity_id == EntityId::Edge(EdgeId::new(2)) && property == db_string("since")
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_closed_preserves_bound_type_for_wal_only() {
    // F2 regression: WAL-only recovery (no snapshot) must accept the
    // caller's bound_type rather than silently defaulting to None.
    // Without this, a closed-graph crash before the first snapshot would
    // permanently downgrade to open and skip GG02 validation forever after.
    let dir = temp_dir("closed-wal-only");
    let graph_type = person_graph_type();
    append_wal(
        &dir,
        0,
        &[Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(db_string("Person")),
            properties: prop("name", Value::String(db_string("Alice"))),
        }],
    );

    let recovered =
        SharedGraph::recover_closed(&dir, GraphId::new(14), graph_type.clone()).unwrap();
    assert!(recovered.is_closed());
    assert_eq!(recovered.graph_type().as_deref(), Some(&graph_type));
    assert!(recovered.read().is_node_alive(NodeId::new(1)));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_closed_rejects_disagreement_with_snapshot_meta() {
    // F2 regression (drift case): if the snapshot's META declares one
    // bound_type and the caller asserts a different one, recovery must fail
    // rather than silently picking either side.
    let dir = temp_dir("closed-drift");
    let snapshot_type = person_graph_type();
    let shared = SharedGraph::builder(GraphId::new(15))
        .bound_to(snapshot_type)
        .unwrap()
        .build()
        .unwrap();
    write_snapshot(&dir, &shared, 1);

    let mut other_type = person_graph_type();
    other_type.name = db_string("closed.person.other");
    let err = match SharedGraph::recover_closed(&dir, GraphId::new(15), other_type) {
        Ok(_) => panic!("recovery should fail on bound_type drift"),
        Err(error) => error,
    };
    let GraphError::Provider(crate::ProviderError::Inconsistent { reason }) = &err else {
        panic!("expected Provider::Inconsistent, got {err:?}");
    };
    assert!(
        reason.contains("bound_type disagrees"),
        "expected bound_type-disagrees, got: {reason}",
    );
    let _ = fs::remove_dir_all(dir);
}
