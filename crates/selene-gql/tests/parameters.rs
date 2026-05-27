//! BRIEF-115 parameter binding integration tests.

use std::{collections::HashMap, num::NonZeroUsize, sync::Arc, thread};

use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, intern};
use selene_gql::{
    AnalysisError, AnalyzedStatement, AnalyzedStatementKind, AnalyzedType, EmptyProcedureRegistry,
    ExecutionPlan, ExecutorError, ExpectedType, GqlStatus, GqlType, LimitValue, OptimizeContext,
    PipelineStatement, Session, Statement, StatementOutput, TypeMismatchContext, ValueExpr,
    analyze, execute_statement, optimize, parse, plan,
};
use selene_graph::{SharedGraph, TypedIndexKind};
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

fn analyze_one(source: &str) -> AnalyzedStatement {
    let statement = parse(source).expect("test source parses");
    analyze(statement, &EmptyProcedureRegistry, None).expect("test source analyzes")
}

fn projection_type(analyzed: &AnalyzedStatement, name: &str) -> AnalyzedType {
    let AnalyzedStatementKind::Query(query) = &analyzed.statement else {
        panic!("expected query statement");
    };
    let item = query
        .statements
        .iter()
        .filter_map(|statement| match statement {
            PipelineStatement::Return(clause) => Some(&clause.items),
            PipelineStatement::With(clause) => Some(&clause.items),
            _ => None,
        })
        .flatten()
        .find(|item| item.alias.is_some_and(|alias| alias.as_str() == name))
        .unwrap_or_else(|| panic!("projection {name} exists"));
    let id = analyzed
        .expr_ids
        .get(&item.expr)
        .unwrap_or_else(|| panic!("projection {name} has an ExprId"));
    analyzed.expr_types.get(id).clone()
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
fn typed_parameter_ast_shapes_cover_value_limit_and_list() {
    let statement = parse("RETURN $id :: INTEGER AS id").expect("typed return parses");
    let Statement::Query(pipeline) = statement else {
        panic!("expected query");
    };
    let PipelineStatement::Return(clause) = &pipeline.statements[0] else {
        panic!("expected RETURN");
    };
    let ValueExpr::Parameter {
        name,
        declared_type: Some(GqlType::Integer),
        ..
    } = &clause.items[0].expr
    else {
        panic!("expected typed parameter expression");
    };
    assert_eq!(name.as_str(), "id");

    let statement = parse("MATCH (n) RETURN n LIMIT $count :: INT").expect("typed LIMIT parses");
    let Statement::Query(pipeline) = statement else {
        panic!("expected query");
    };
    let PipelineStatement::Limit(LimitValue::Parameter {
        name,
        declared_type: Some(GqlType::Integer),
        ..
    }) = &pipeline.statements[2]
    else {
        panic!("expected typed LIMIT parameter");
    };
    assert_eq!(name.as_str(), "count");

    let statement = parse("RETURN $vals :: LIST<INT> AS vals").expect("typed list parses");
    let Statement::Query(pipeline) = statement else {
        panic!("expected query");
    };
    let PipelineStatement::Return(clause) = &pipeline.statements[0] else {
        panic!("expected RETURN");
    };
    let ValueExpr::Parameter {
        declared_type: Some(GqlType::List(inner)),
        ..
    } = &clause.items[0].expr
    else {
        panic!("expected LIST typed parameter expression");
    };
    assert_eq!(**inner, GqlType::Integer);
}

#[test]
fn untyped_parameter_ast_and_analyzer_stay_dynamic() {
    let statement = parse("RETURN $id AS id").expect("untyped parameter parses");
    let Statement::Query(pipeline) = &statement else {
        panic!("expected query");
    };
    let PipelineStatement::Return(clause) = &pipeline.statements[0] else {
        panic!("expected RETURN");
    };
    assert!(matches!(
        &clause.items[0].expr,
        ValueExpr::Parameter {
            name,
            declared_type: None,
            ..
        } if name.as_str() == "id"
    ));

    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("source analyzes");
    assert_eq!(projection_type(&analyzed, "id"), AnalyzedType::Dynamic);
}

#[test]
fn typed_parameter_declaration_inherits_to_bare_references() {
    let analyzed = analyze_one("RETURN $x :: INT AS typed, $x AS bare");
    assert_eq!(
        projection_type(&analyzed, "typed"),
        AnalyzedType::Resolved(GqlType::Integer)
    );
    assert_eq!(
        projection_type(&analyzed, "bare"),
        AnalyzedType::Resolved(GqlType::Integer)
    );
}

#[test]
fn typed_parameter_conflicts_are_analyzer_errors() {
    let err = analyze(
        parse("MATCH (n {a: $x :: INT, b: $x :: STRING}) RETURN n").expect("source parses"),
        &EmptyProcedureRegistry,
        None,
    )
    .expect_err("conflicting declarations reject");
    assert!(matches!(
        err,
        AnalysisError::ConflictingParameterTypes {
            name,
            ref declarations,
        } if name.as_str() == "x"
            && declarations.len() == 2
            && declarations[0].0 == GqlType::Integer
            && declarations[1].0 == GqlType::String
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::DATATYPE_MISMATCH);
}

#[test]
fn typed_parameter_conflicts_cross_value_and_limit_surfaces() {
    let err = analyze(
        parse("MATCH (n {a: $x :: INT}) RETURN n LIMIT $x :: STRING").expect("source parses"),
        &EmptyProcedureRegistry,
        None,
    )
    .expect_err("cross-surface conflict rejects");
    assert!(matches!(
        err,
        AnalysisError::ConflictingParameterTypes { name, .. } if name.as_str() == "x"
    ));
}

#[test]
fn typed_limit_parameter_rejects_unsatisfiable_declared_types_at_analysis() {
    for source in [
        "RETURN 1 AS n LIMIT $rows :: STRING",
        "RETURN 1 AS n OFFSET $rows :: FLOAT",
    ] {
        let err = analyze(
            parse(source).expect("source parses"),
            &EmptyProcedureRegistry,
            None,
        )
        .expect_err("unsatisfiable LIMIT/OFFSET declaration rejects");
        assert!(matches!(
            err,
            AnalysisError::TypeMismatch {
                context: TypeMismatchContext::LimitAmount,
                expected: ExpectedType::LimitAmount,
                ..
            }
        ));
        assert_eq!(err.gqlstatus(), GqlStatus::DATATYPE_MISMATCH);
    }
}

#[test]
fn typed_parameter_runtime_rejects_bare_reference_inherited_from_declaration() {
    let graph = SharedGraph::new(GraphId::new(4120));
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("x"), Value::String(istr("abc")));

    let err = execute(
        &mut session,
        "RETURN CASE WHEN false THEN $x :: INT ELSE $x END AS value",
    )
    .expect_err("bare reference inherits declared type");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name,
            ref expected,
            actual: "STRING",
            ..
        } if name.as_str() == "x" && expected == "INTEGER"
    ));
}

#[test]
fn typed_parameter_runtime_accepts_and_rejects_declared_value_type() {
    let graph = SharedGraph::new(GraphId::new(4117));
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("id"), Value::Int(42));
    assert_eq!(
        single_value(&mut session, "RETURN $id :: INT AS id"),
        Value::Int(42)
    );

    session.bind_parameter(istr("id"), Value::String(istr("abc")));
    let err = execute(&mut session, "RETURN $id :: INT AS id")
        .expect_err("typed parameter rejects mismatched value");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name,
            ref expected,
            actual: "STRING",
            ..
        } if name.as_str() == "id" && expected == "INTEGER"
    ));
}

#[test]
fn typed_parameter_python_shape_fixture() {
    let graph = SharedGraph::new(GraphId::new(4121));
    let mut session = Session::new(&graph);
    let mut params = HashMap::new();
    params.insert("id", serde_json::json!(42));
    params.insert("name", serde_json::json!("thermostat"));

    bind_json_params(&mut session, params);

    let table = rows(
        execute(
            &mut session,
            "INSERT (n:Sensor {id: $id :: INT, name: $name :: STRING}) \
             RETURN n.id AS id, n.name AS name",
        )
        .expect("typed parameter insert executes"),
    );
    assert_eq!(
        table.rows()[0].values(),
        &[Value::Int(42), Value::String(istr("thermostat"))]
    );
}

#[test]
fn typed_limit_parameter_runtime_checks_declared_type_first() {
    let graph = graph_with_sensors(4118);
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("count"), Value::Float(3.0));

    let err = execute(
        &mut session,
        "MATCH (n:Sensor) RETURN n LIMIT $count :: INT",
    )
    .expect_err("typed LIMIT parameter rejects mismatched value");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name,
            ref expected,
            actual: "FLOAT64",
            ..
        } if name.as_str() == "count" && expected == "INTEGER"
    ));
}

#[test]
fn typed_parameter_plan_cache_reuses_same_type_and_partitions_different_types() {
    let graph = SharedGraph::new(GraphId::new(4119));
    let mut session = Session::new(&graph).with_plan_cache(NonZeroUsize::new(8).unwrap());
    session.bind_parameter(istr("a"), Value::Int(1));
    assert_eq!(
        single_value(&mut session, "RETURN $a :: INT AS a"),
        Value::Int(1)
    );
    session.bind_parameter(istr("a"), Value::Int(2));
    assert_eq!(
        single_value(&mut session, "RETURN $a :: INT AS a"),
        Value::Int(2)
    );
    session.bind_parameter(istr("a"), Value::String(istr("x")));
    assert_eq!(
        single_value(&mut session, "RETURN $a :: STRING AS a"),
        Value::String(istr("x"))
    );

    let stats = session.plan_cache_stats().expect("cache enabled");
    assert_eq!(stats.misses, 2);
    assert_eq!(stats.hits, 1);
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

#[test]
fn external_string_parameter_lands_in_indexed_string_column() {
    // BRIEF-153: bind a `Value::ExternalString` to a `$p :: STRING`
    // parameter, INSERT against a `STRING NOT NULL INDEXED` schema, and
    // assert the row is reachable through the property index from both
    // the `Value::String` (admitted) and `Value::ExternalString` (raw)
    // probe variants. Closes the silent-skip footgun for downstream
    // consumers who bind ExternalString through GQL.
    use selene_core::lookup;

    let graph = SharedGraph::new(GraphId::new(4900));
    let function_label = istr("Brief153Function");
    let qualified_name = istr("qualified_name");
    graph
        .create_property_index(function_label, qualified_name, TypedIndexKind::String)
        .expect("registers");

    let probe = "brief153::ext::unique_qualified_name";
    assert!(
        lookup(probe).is_none(),
        "fresh probe content must not already be in the IStr pool"
    );

    let mut session = Session::new(&graph);
    session.bind_parameter(istr("p"), Value::ExternalString(Arc::<str>::from(probe)));
    let table = rows(
        execute(
            &mut session,
            "INSERT (n:Brief153Function {qualified_name: $p :: STRING}) \
             RETURN n.qualified_name AS qn",
        )
        .expect("typed-parameter insert executes"),
    );
    assert_eq!(table.row_count(), 1);

    // After commit the IStr is in the pool (admission landed at the
    // index-commit boundary) and the index returns the new row from
    // either probe variant.
    let admitted = lookup(probe).expect("commit admitted the IStr");
    let snapshot = graph.read();
    let rows_via_string = snapshot
        .nodes_with_property_eq(&function_label, &qualified_name, &Value::String(admitted))
        .expect("kind matches");
    let rows_via_external = snapshot
        .nodes_with_property_eq(
            &function_label,
            &qualified_name,
            &Value::ExternalString(Arc::<str>::from(probe)),
        )
        .expect("kind matches");
    assert!(!rows_via_string.is_empty());
    assert!(!rows_via_external.is_empty());
    assert_eq!(
        rows_via_string.iter().collect::<Vec<_>>(),
        rows_via_external.iter().collect::<Vec<_>>(),
    );
}
