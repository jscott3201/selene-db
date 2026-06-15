use super::*;

#[test]
fn plan_cache_hits_across_param_value_changes() {
    let graph = graph_with_sensors(4114);
    let mut session = Session::new(&graph).with_plan_cache(NonZeroUsize::new(8).unwrap());
    let id = db_string("id");
    let source = "MATCH (n {id: $id}) RETURN n";
    session.bind_parameter(id.clone(), Value::Int(1));
    assert_eq!(rows(execute(&mut session, source).unwrap()).row_count(), 1);

    session.bind_parameter(id, Value::Int(2));
    assert_eq!(rows(execute(&mut session, source).unwrap()).row_count(), 1);

    let stats = session.plan_cache_stats().expect("cache enabled");
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hits, 1);
}

#[test]
fn plan_cache_runtime_type_check_with_dynamic() {
    let graph = graph_with_sensors(4115);
    let mut session = Session::new(&graph).with_plan_cache(NonZeroUsize::new(8).unwrap());
    let id = db_string("id");
    let source = "MATCH (n {id: $id}) RETURN n";
    session.bind_parameter(id.clone(), Value::Int(1));
    assert_eq!(rows(execute(&mut session, source).unwrap()).row_count(), 1);

    session.bind_parameter(id, Value::String(db_string("not-present")));
    assert_eq!(rows(execute(&mut session, source).unwrap()).row_count(), 0);

    let stats = session.plan_cache_stats().expect("cache enabled");
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hits, 1);
}

#[test]
fn python_shape_fixture() {
    let graph = SharedGraph::new(GraphId::new(4116));
    let mut session = Session::new(&graph);
    let mut params = HashMap::new();
    params.insert("id", serde_json::json!(42));
    params.insert("name", serde_json::json!("thermostat"));

    bind_json_params(&mut session, params);

    let table = rows(
        execute(
            &mut session,
            "INSERT (n:Sensor {id: $id, name: $name}) RETURN n.id AS id, n.name AS name",
        )
        .expect("insert executes"),
    );
    assert_eq!(
        table.rows()[0].values(),
        &[Value::Int(42), Value::String(db_string("thermostat"))]
    );
}
