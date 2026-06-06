//! Implementation-defined JSON value and scalar function coverage.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, db_string, execute_read, execute_read_result};
use selene_core::{
    GraphId, JsonValue as CoreJsonValue, PropertyValueType, Value, feature_register::FeatureId,
};
use selene_gql::{
    EmptyProcedureRegistry, ExecutorError, GqlType, PipelineStatement, QueryPipeline, ReturnClause,
    ReturnItem, Session, SourceSpan, Statement, StatementOutput, ValueExpr, feature_walk, parse,
};
use selene_graph::{GraphTypeDef, SharedGraph};

fn single_value(source: &str, column: &str) -> Value {
    let table = execute_read(source);
    let mut values = column_values(&table, column);
    assert_eq!(values.len(), 1);
    values.pop().expect("one row")
}

fn json_value(source: &str) -> CoreJsonValue {
    match single_value(source, "value") {
        Value::Json(value) => value,
        other => panic!("expected JSON value, got {other:?}"),
    }
}

fn string_value(source: &str) -> String {
    match single_value(source, "value") {
        Value::String(value) => value.to_string(),
        other => panic!("expected STRING value, got {other:?}"),
    }
}

fn assert_status(source: &str, expected: &str) {
    let err = execute_read_result(source).expect_err("query should fail");
    assert_eq!(err.gqlstatus().as_str(), expected, "source: {source}");
}

fn assert_feature_recorded(source: &str) {
    let statement = parse(source).expect(source);
    let observed = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();
    assert!(
        observed.contains(&FeatureId::IM_JSON),
        "{source} should record IM_JSON, observed {observed:?}"
    );
}

fn typed_parameter_statement(name: selene_core::DbString) -> Statement {
    let span = SourceSpan::new(0, 4);
    Statement::Query(QueryPipeline {
        statements: vec![PipelineStatement::Return(ReturnClause {
            distinct: false,
            star: false,
            items: vec![ReturnItem {
                expr: ValueExpr::Parameter {
                    name: name.clone(),
                    declared_type: Some(GqlType::Json),
                    span,
                },
                alias: Some(name),
                span,
            }],
            group_by: None,
            having: None,
            span,
        })],
        span,
    })
}

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: db_string("json.test.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .expect("graph type binds")
        .build()
        .expect("graph builds")
}

fn rows_from_output(output: StatementOutput) -> selene_gql::BindingTable {
    let StatementOutput::Rows(table) = output else {
        panic!("expected row output");
    };
    table
}

#[test]
fn json_parse_returns_canonical_json_value() {
    let value = json_value(r#"RETURN json('{"b":[2,true],"a":null}') AS value"#);
    assert_eq!(value.to_canonical_string(), r#"{"a":null,"b":[2,true]}"#);
}

#[test]
fn json_stringify_and_cast_to_string_use_canonical_form() {
    assert_eq!(
        string_value(r#"RETURN json_stringify(json('{"b":2,"a":1}')) AS value"#),
        r#"{"a":1,"b":2}"#
    );
    assert_eq!(
        string_value(r#"RETURN CAST(json('{"b":2,"a":1}') AS STRING) AS value"#),
        r#"{"a":1,"b":2}"#
    );
}

#[test]
fn json_type_reports_top_level_shape() {
    for (source, expected) in [
        (r#"RETURN json_type(json('null')) AS value"#, "null"),
        (r#"RETURN json_type(json('true')) AS value"#, "boolean"),
        (r#"RETURN json_type(json('1.5')) AS value"#, "number"),
        (r#"RETURN json_type(json('"agent"')) AS value"#, "string"),
        (r#"RETURN json_type(json('[1]')) AS value"#, "array"),
        (r#"RETURN json_type(json('{"a":1}')) AS value"#, "object"),
    ] {
        assert_eq!(string_value(source), expected, "{source}");
    }
}

#[test]
fn json_get_selects_objects_and_arrays() {
    let value = json_value(
        r#"RETURN json_get(json('{"memory":{"score":7,"kind":"episodic"}}'), 'memory') AS value"#,
    );
    assert_eq!(
        value.to_canonical_string(),
        r#"{"kind":"episodic","score":7}"#
    );
    assert_eq!(
        string_value(r#"RETURN json_get_text(json('{"name":"alpha"}'), 'name') AS value"#),
        "alpha"
    );
    assert_eq!(
        string_value(r#"RETURN json_get_text(json('{"memory":{"score":7}}'), 'memory') AS value"#),
        r#"{"score":7}"#
    );
    assert_eq!(
        string_value(r#"RETURN json_get_text(json('[10,20,30]'), 1) AS value"#),
        "20"
    );
    assert_eq!(
        string_value(r#"RETURN json_get_text(json('[10,20,30]'), -1) AS value"#),
        "30"
    );
}

#[test]
fn json_get_returns_sql_null_for_absent_or_inapplicable_paths() {
    for source in [
        r#"RETURN json_get(json('{"a":1}'), 'missing') AS value"#,
        r#"RETURN json_get(json('{"a":1}'), NULL) AS value"#,
        r#"RETURN json_get(NULL, 'a') AS value"#,
        r#"RETURN json_get(json('[1,2]'), 99) AS value"#,
        r#"RETURN json_get(json('"not-container"'), 'a') AS value"#,
        r#"RETURN json_get_text(json('{"a":null}'), 'a') AS value"#,
    ] {
        assert_eq!(single_value(source, "value"), Value::Null, "{source}");
    }
}

#[test]
fn json_equality_uses_value_semantics_but_ordering_is_rejected() {
    assert_eq!(
        single_value(
            r#"RETURN json('{"b":2,"a":1}') = json('{"a":1,"b":2}') AS value"#,
            "value",
        ),
        Value::Bool(true)
    );
    assert_status(
        r#"RETURN json('{"a":1}') < json('{"a":2}') AS value"#,
        "22G04",
    );
}

#[test]
fn json_functions_report_data_exceptions_for_bad_inputs() {
    for source in [
        "RETURN json('{') AS value",
        "RETURN CAST('[' AS JSON) AS value",
    ] {
        assert_status(source, "22018");
    }
    for source in [
        "RETURN json(7) AS value",
        "RETURN json_stringify(7) AS value",
        "RETURN json_type(7) AS value",
        r#"RETURN json_get(json('{"a":1}'), 7) AS value"#,
        r#"RETURN json_get(json('[1,2]'), 'bad') AS value"#,
        "RETURN CAST(7 AS JSON) AS value",
        "RETURN CAST(json('{}') AS INTEGER) AS value",
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn json_feature_flags_cover_functions_and_type_names() {
    for source in [
        r#"RETURN json('{"a":1}') AS value"#,
        r#"RETURN json_parse('{"a":1}') AS value"#,
        r#"RETURN json_stringify(json('{"a":1}')) AS value"#,
        r#"RETURN json_type(json('{"a":1}')) AS value"#,
        r#"RETURN json_get(json('{"a":1}'), 'a') AS value"#,
        r#"RETURN json_get_text(json('{"a":1}'), 'a') AS value"#,
        "RETURN NULL IS TYPED JSON AS value",
        "CREATE NODE TYPE :Thing (payload :: JSON)",
    ] {
        assert_feature_recorded(source);
    }
}

#[test]
fn json_typed_parameters_accept_json_values_and_reject_other_values() {
    let name = db_string("doc");
    let statement = typed_parameter_statement(name.clone());
    let analyzed =
        selene_gql::analyze(statement, &EmptyProcedureRegistry, None).expect("statement analyzes");
    let plan = selene_gql::plan(&analyzed, &EmptyProcedureRegistry).expect("statement plans");
    let graph = SharedGraph::new(GraphId::new(14_001));
    let mut session = Session::new(&graph);

    session.bind_parameter(
        name.clone(),
        Value::Json(CoreJsonValue::new(serde_json::json!({"a": 1})).expect("JSON is valid")),
    );
    selene_gql::execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
        .expect("JSON parameter accepts matching value");

    session.bind_parameter(name.clone(), Value::String(db_string("{}")));
    let err = selene_gql::execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
        .expect_err("JSON parameter rejects string value");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name: err_name,
            ref expected,
            actual: "STRING",
            ..
        } if err_name == name && expected == "JSON"
    ));
}

#[test]
fn is_typed_json_matches_json_values() {
    assert_eq!(
        single_value(r#"RETURN json('{"a":1}') IS TYPED JSON AS value"#, "value"),
        Value::Bool(true)
    );
    assert_eq!(
        single_value(r#"RETURN '{"a":1}' IS TYPED JSON AS value"#, "value"),
        Value::Bool(false)
    );
}

#[test]
fn json_node_type_round_trips_catalog_and_execution() {
    let graph = empty_closed_graph(14_002);
    let mut session = Session::new(&graph);
    session
        .execute_source(
            "CREATE NODE TYPE :Thing (payload :: JSON)",
            &EmptyProcedureRegistry,
        )
        .expect("JSON node type creates");

    let graph_type = graph.graph_type().expect("graph has type");
    let declaration = &graph_type.node_types[0].properties[0];
    assert_eq!(declaration.value_type, PropertyValueType::Json);

    session
        .execute_source(
            r#"INSERT (:Thing {payload: json('{"name":"alpha","score":7}')})"#,
            &EmptyProcedureRegistry,
        )
        .expect("JSON property inserts");
    let output = session
        .execute_source(
            "MATCH (n:Thing) RETURN json_get_text(n.payload, 'score') AS value",
            &EmptyProcedureRegistry,
        )
        .expect("JSON property reads");
    let table = rows_from_output(output);
    assert_eq!(
        column_values(&table, "value"),
        vec![Value::String(db_string("7"))]
    );

    let output = session
        .execute_source("SHOW NODE TYPES", &EmptyProcedureRegistry)
        .expect("SHOW NODE TYPES executes");
    let table = rows_from_output(output);
    assert_eq!(
        column_values(&table, "definition"),
        vec![Value::String(db_string(
            "CREATE NODE TYPE :Thing (payload :: JSON)"
        ))]
    );
}
