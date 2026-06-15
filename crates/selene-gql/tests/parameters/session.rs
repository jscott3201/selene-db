use super::*;

#[test]
fn per_session_isolation() {
    let graph = Arc::new(SharedGraph::new(GraphId::new(4108)));
    let first_graph = Arc::clone(&graph);
    let second_graph = Arc::clone(&graph);

    let first = thread::spawn(move || {
        let mut session = Session::new(&first_graph);
        session.bind_parameter(db_string("id"), Value::Int(1));
        for _ in 0..25 {
            assert_eq!(
                single_value(&mut session, "RETURN $id AS id"),
                Value::Int(1)
            );
        }
    });
    let second = thread::spawn(move || {
        let mut session = Session::new(&second_graph);
        session.bind_parameter(db_string("id"), Value::Int(2));
        for _ in 0..25 {
            assert_eq!(
                single_value(&mut session, "RETURN $id AS id"),
                Value::Int(2)
            );
        }
    });

    first.join().expect("first thread succeeds");
    second.join().expect("second thread succeeds");
}

#[test]
fn mutation_parameters() {
    let graph = SharedGraph::new(GraphId::new(4109));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("id"), Value::Int(42));
    session.bind_parameter(db_string("name"), Value::String(db_string("thermostat")));

    let table = rows(
        execute(
            &mut session,
            "INSERT (n:Sensor {id: $id, name: $name}) RETURN n.id AS id, n.name AS name",
        )
        .expect("insert executes"),
    );

    assert_eq!(table.row_count(), 1);
    assert_eq!(
        table.rows()[0].values(),
        &[Value::Int(42), Value::String(db_string("thermostat"))]
    );
}

#[test]
fn rebinding_is_upsert() {
    let graph = SharedGraph::new(GraphId::new(4110));
    let mut session = Session::new(&graph);
    let id = db_string("id");

    assert_eq!(session.bind_parameter(id.clone(), Value::Int(1)), None);
    assert_eq!(
        single_value(&mut session, "RETURN $id AS id"),
        Value::Int(1)
    );
    assert_eq!(
        session.bind_parameter(id, Value::Int(2)),
        Some(Value::Int(1))
    );
    assert_eq!(
        single_value(&mut session, "RETURN $id AS id"),
        Value::Int(2)
    );
}

#[test]
fn clear_parameter_apis_remove_bindings() {
    let graph = SharedGraph::new(GraphId::new(4111));
    let mut session = Session::new(&graph);
    let id = db_string("id");
    let name = db_string("name");

    session.bind_parameter(id.clone(), Value::Int(1));
    session.bind_parameter(name, Value::String(db_string("sensor")));
    assert_eq!(session.clear_parameter(&id), Some(Value::Int(1)));
    assert!(matches!(
        execute(&mut session, "RETURN $id AS id"),
        Err(ExecutorError::UnboundParameter { .. })
    ));

    session.clear_parameters();
    assert!(matches!(
        execute(&mut session, "RETURN $name AS name"),
        Err(ExecutorError::UnboundParameter { .. })
    ));
}

#[test]
fn parameters_preserved_across_tx_boundaries() {
    let graph = SharedGraph::new(GraphId::new(4112));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("id"), Value::Int(42));

    assert_eq!(
        single_value(&mut session, "RETURN $id AS id"),
        Value::Int(42)
    );

    session.start_transaction().expect("start succeeds");
    assert_eq!(
        single_value(&mut session, "RETURN $id AS id"),
        Value::Int(42)
    );
    session.commit_transaction().expect("commit succeeds");
    assert_eq!(
        single_value(&mut session, "RETURN $id AS id"),
        Value::Int(42)
    );

    session.start_transaction().expect("start succeeds");
    assert_eq!(
        single_value(&mut session, "RETURN $id AS id"),
        Value::Int(42)
    );
    session.rollback_transaction().expect("rollback succeeds");
    assert_eq!(
        single_value(&mut session, "RETURN $id AS id"),
        Value::Int(42)
    );

    session.start_transaction().expect("start succeeds");
    assert_eq!(
        single_value(&mut session, "RETURN $id AS id"),
        Value::Int(42)
    );
    session.abort();
    assert_eq!(
        single_value(&mut session, "RETURN $id AS id"),
        Value::Int(42)
    );
}

#[test]
fn runtime_error_in_tx_sets_aborted() {
    let graph = SharedGraph::new(GraphId::new(4113));
    let mut session = Session::new(&graph);
    session.start_transaction().expect("start succeeds");

    let err = execute(&mut session, "RETURN $missing AS value").expect_err("param is unbound");

    assert!(matches!(err, ExecutorError::UnboundParameter { .. }));
    assert!(session.is_aborted());
    assert!(matches!(
        execute(&mut session, "RETURN 1 AS value"),
        Err(ExecutorError::InFailedTransaction { .. })
    ));
    let rollback = session.rollback_transaction().expect("rollback succeeds");
    assert_eq!(rollback.statement_count, 0);
}
