use super::*;

#[test]
fn bind_parameter_per_value_variant() {
    let graph = SharedGraph::new(GraphId::new(4100));
    let mut session = Session::new(&graph);

    for (index, make_value) in Value::ALL.iter().enumerate() {
        let name = db_string(&format!("p{index}"));
        let value = make_value();
        session.bind_parameter(name.clone(), value.clone());

        let returned = single_value(&mut session, &format!("RETURN ${name} AS value"));

        assert_eq!(returned, value);
    }
}

#[test]
fn limit_parameter_int() {
    let graph = graph_with_sensors(4101);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("count"), Value::Int(3));

    let table = rows(execute(&mut session, "MATCH (n:Sensor) RETURN n LIMIT $count").unwrap());

    assert_eq!(table.row_count(), 3);
}

#[test]
fn limit_parameter_negative_rejected() {
    let graph = graph_with_sensors(4102);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("count"), Value::Int(-1));

    let err = execute(&mut session, "MATCH (n:Sensor) RETURN n LIMIT $count")
        .expect_err("negative limit is rejected");

    assert!(matches!(
        err,
        ExecutorError::DataException {
            subclass: DataExceptionSubclass::NegativeLimitValue,
            ..
        }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::NEGATIVE_LIMIT_VALUE);
}

#[test]
fn limit_parameter_non_integer_rejected() {
    let graph = graph_with_sensors(4103);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("count"), Value::String(db_string("five")));

    let err = execute(&mut session, "MATCH (n:Sensor) RETURN n LIMIT $count")
        .expect_err("string limit is rejected");

    assert!(matches!(
        err,
        ExecutorError::DataException {
            subclass: DataExceptionSubclass::InvalidValueType,
            ..
        }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::DATATYPE_MISMATCH);
}

#[test]
fn order_by_limit_parameter_uses_top_k_path() {
    let graph = graph_with_sensors(4104);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("count"), Value::Int(2));
    let plan =
        optimized_plan("MATCH (n:Sensor) RETURN n.value AS value ORDER BY value DESC LIMIT $count");

    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
            .expect("optimized statement executes"),
    );

    let values = table
        .rows()
        .iter()
        .map(|row| row.values()[0].clone())
        .collect::<Vec<_>>();
    assert_eq!(values, vec![Value::Int(70), Value::Int(60)]);
}

#[test]
fn property_match_parameter_polymorphic() {
    let graph = graph_with_sensors(4105);
    let mut session = Session::new(&graph);
    let id = db_string("id");
    let uuid_value = sample_uuid_value();

    for value in [
        Value::Int(1),
        Value::String(db_string("string-id")),
        uuid_value.clone(),
    ] {
        session.bind_parameter(id.clone(), value);
        let table = rows(
            execute(&mut session, "MATCH (n {id: $id}) RETURN n")
                .expect("parameterized property match succeeds"),
        );
        assert_eq!(table.row_count(), 1);
    }
}

#[test]
fn unbound_parameter_error_carries_span() {
    let graph = SharedGraph::new(GraphId::new(4106));
    let mut session = Session::new(&graph);

    let err = execute(&mut session, "RETURN $missing AS value").expect_err("param is unbound");

    assert!(matches!(
        err,
        ExecutorError::UnboundParameter { name, span }
            if name.as_str() == "missing" && span.byte_offset == 7 && span.byte_len == 8
    ));
}

#[test]
fn unreferenced_parameter_is_lenient() {
    let graph = SharedGraph::new(GraphId::new(4107));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("unused"), Value::Int(1));

    assert_eq!(
        single_value(&mut session, "RETURN 42 AS value"),
        Value::Int(42)
    );
}
