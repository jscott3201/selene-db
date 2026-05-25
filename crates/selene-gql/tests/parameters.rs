//! BRIEF-115 parameter binding integration tests.

use std::{collections::HashMap, num::NonZeroUsize, sync::Arc, thread};

use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, intern};
use selene_gql::{
    EmptyProcedureRegistry, ExecutionPlan, ExecutorError, GqlStatus, OptimizeContext, Session,
    StatementOutput, analyze, execute_statement, optimize, parse, plan,
};
use selene_graph::SharedGraph;
use serde_json::Value as JsonValue;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn props<const N: usize>(pairs: [(IStr, Value); N]) -> PropertyMap {
    PropertyMap::from_pairs(pairs).expect("test properties fit caps")
}

fn execute(session: &mut Session<'_>, source: &str) -> Result<StatementOutput, ExecutorError> {
    session.execute_source(source, &EmptyProcedureRegistry)
}

fn planned(source: &str) -> ExecutionPlan {
    let statement = parse(source).expect("test source parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test source analyzes");
    plan(&analyzed, &EmptyProcedureRegistry).expect("test source plans")
}

fn optimized_plan(source: &str) -> ExecutionPlan {
    optimize(planned(source), &OptimizeContext::default())
}

fn rows(output: StatementOutput) -> selene_gql::BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("write returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn single_value(session: &mut Session<'_>, source: &str) -> Value {
    let table = rows(execute(session, source).expect("query succeeds"));
    assert_eq!(table.row_count(), 1);
    table.rows()[0].values()[0].clone()
}

fn sample_uuid_value() -> Value {
    Value::ALL
        .iter()
        .map(|make| make())
        .find(|value| matches!(value, Value::Uuid(_)))
        .expect("Value::ALL includes uuid")
}

fn graph_with_sensors(id: u64) -> SharedGraph {
    let graph = SharedGraph::new(GraphId::new(id));
    let sensor = istr("Sensor");
    let id_key = istr("id");
    let value_key = istr("value");
    let name_key = istr("name");
    let uuid_value = sample_uuid_value();
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for index in 0_i64..5 {
            mutator
                .create_node(
                    LabelSet::single(sensor),
                    props([
                        (id_key, Value::Int(index)),
                        (value_key, Value::Int(index * 10)),
                        (name_key, Value::String(istr(&format!("sensor-{index}")))),
                    ]),
                )
                .expect("sensor inserts");
        }
        mutator
            .create_node(
                LabelSet::single(sensor),
                props([
                    (id_key, Value::String(istr("string-id"))),
                    (value_key, Value::Int(60)),
                ]),
            )
            .expect("string-id sensor inserts");
        mutator
            .create_node(
                LabelSet::single(sensor),
                props([(id_key, uuid_value), (value_key, Value::Int(70))]),
            )
            .expect("uuid-id sensor inserts");
        txn.commit().expect("fixture commits");
    }
    graph
}

#[test]
fn bind_parameter_per_value_variant() {
    let graph = SharedGraph::new(GraphId::new(4100));
    let mut session = Session::new(&graph);

    for (index, make_value) in Value::ALL.iter().enumerate() {
        let name = istr(&format!("p{index}"));
        let value = make_value();
        session.bind_parameter(name, value.clone());

        let returned = single_value(&mut session, &format!("RETURN ${name} AS value"));

        assert_eq!(returned, value);
    }
}

#[test]
fn limit_parameter_int() {
    let graph = graph_with_sensors(4101);
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("count"), Value::Int(3));

    let table = rows(execute(&mut session, "MATCH (n:Sensor) RETURN n LIMIT $count").unwrap());

    assert_eq!(table.row_count(), 3);
}

#[test]
fn limit_parameter_negative_rejected() {
    let graph = graph_with_sensors(4102);
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("count"), Value::Int(-1));

    let err = execute(&mut session, "MATCH (n:Sensor) RETURN n LIMIT $count")
        .expect_err("negative limit is rejected");

    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            ref expected,
            actual: "negative integer",
            ..
        } if expected == "non-negative integer"
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::INVALID_PROCEDURE_ARGUMENT);
}

#[test]
fn limit_parameter_non_integer_rejected() {
    let graph = graph_with_sensors(4103);
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("count"), Value::String(istr("five")));

    let err = execute(&mut session, "MATCH (n:Sensor) RETURN n LIMIT $count")
        .expect_err("string limit is rejected");

    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            ref expected,
            actual: "string",
            ..
        } if expected == "non-negative integer"
    ));
}

#[test]
fn order_by_limit_parameter_uses_top_k_path() {
    let graph = graph_with_sensors(4104);
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("count"), Value::Int(2));
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
    let id = istr("id");
    let uuid_value = sample_uuid_value();

    for value in [
        Value::Int(1),
        Value::String(istr("string-id")),
        uuid_value.clone(),
    ] {
        session.bind_parameter(id, value);
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
    session.bind_parameter(istr("unused"), Value::Int(1));

    assert_eq!(
        single_value(&mut session, "RETURN 42 AS value"),
        Value::Int(42)
    );
}

#[test]
fn per_session_isolation() {
    let graph = Arc::new(SharedGraph::new(GraphId::new(4108)));
    let first_graph = Arc::clone(&graph);
    let second_graph = Arc::clone(&graph);

    let first = thread::spawn(move || {
        let mut session = Session::new(&first_graph);
        session.bind_parameter(istr("id"), Value::Int(1));
        for _ in 0..25 {
            assert_eq!(
                single_value(&mut session, "RETURN $id AS id"),
                Value::Int(1)
            );
        }
    });
    let second = thread::spawn(move || {
        let mut session = Session::new(&second_graph);
        session.bind_parameter(istr("id"), Value::Int(2));
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
    session.bind_parameter(istr("id"), Value::Int(42));
    session.bind_parameter(istr("name"), Value::String(istr("thermostat")));

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
        &[Value::Int(42), Value::String(istr("thermostat"))]
    );
}

#[test]
fn rebinding_is_upsert() {
    let graph = SharedGraph::new(GraphId::new(4110));
    let mut session = Session::new(&graph);
    let id = istr("id");

    assert_eq!(session.bind_parameter(id, Value::Int(1)), None);
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
    let id = istr("id");
    let name = istr("name");

    session.bind_parameter(id, Value::Int(1));
    session.bind_parameter(name, Value::String(istr("sensor")));
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
    session.bind_parameter(istr("id"), Value::Int(42));

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

#[test]
fn plan_cache_hits_across_param_value_changes() {
    let graph = graph_with_sensors(4114);
    let mut session = Session::new(&graph).with_plan_cache(NonZeroUsize::new(8).unwrap());
    let id = istr("id");
    let source = "MATCH (n {id: $id}) RETURN n";
    session.bind_parameter(id, Value::Int(1));
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
    let id = istr("id");
    let source = "MATCH (n {id: $id}) RETURN n";
    session.bind_parameter(id, Value::Int(1));
    assert_eq!(rows(execute(&mut session, source).unwrap()).row_count(), 1);

    session.bind_parameter(id, Value::String(istr("not-present")));
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
        &[Value::Int(42), Value::String(istr("thermostat"))]
    );
}

fn bind_json_params(session: &mut Session<'_>, params: HashMap<&str, JsonValue>) {
    for (name, value) in params {
        session.bind_parameter(istr(name), json_to_value(value));
    }
}

fn json_to_value(value: JsonValue) -> Value {
    match value {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(value) => Value::Bool(value),
        JsonValue::Number(value) => {
            if let Some(value) = value.as_i64() {
                Value::Int(value)
            } else if let Some(value) = value.as_u64() {
                Value::Uint(value)
            } else {
                Value::Float(value.as_f64().expect("json number is finite"))
            }
        }
        JsonValue::String(value) => Value::String(istr(&value)),
        JsonValue::Array(values) => Value::List(values.into_iter().map(json_to_value).collect()),
        JsonValue::Object(_) => Value::Null,
    }
}
