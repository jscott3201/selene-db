//! Focused coverage for RFC 6902 JSON Patch helpers.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, db_string, execute_read_result};
use selene_core::{GraphId, JsonValue as CoreJsonValue, PropertyValueType, Value};
use selene_gql::{EmptyProcedureRegistry, Session, StatementOutput};
use selene_graph::{GraphTypeDef, SharedGraph};

fn single_value(source: &str, column: &str) -> Value {
    let output = execute_read_result(source).expect("query executes");
    let mut values = column_values(&output, column);
    assert_eq!(values.len(), 1);
    values.pop().expect("one row")
}

fn json_value(source: &str) -> CoreJsonValue {
    match single_value(source, "value") {
        Value::Json(value) => value,
        other => panic!("expected JSON value, got {other:?}"),
    }
}

fn assert_status(source: &str, expected: &str) {
    let err = execute_read_result(source).expect_err("query should fail");
    assert_eq!(err.gqlstatus().as_str(), expected, "source: {source}");
}

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: db_string("json.patch.graph"),
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
fn json_patch_applies_rfc6902_operation_sequence() {
    let value = json_value(
        r#"RETURN json_patch(
            json('{"foo":"bar","items":["all","grass","cows","eat"],"source":{"keep":true,"move":"value"}}'),
            json('[
                {"op":"test","path":"/foo","value":"bar"},
                {"op":"add","path":"/items/1","value":"green"},
                {"op":"remove","path":"/items/3"},
                {"op":"replace","path":"/foo","value":"baz"},
                {"op":"copy","from":"/source/keep","path":"/copied"},
                {"op":"move","from":"/source/move","path":"/moved"}
            ]')
        ) AS value"#,
    );

    assert_eq!(
        value.to_canonical_string(),
        r#"{"copied":true,"foo":"baz","items":["all","green","grass","eat"],"moved":"value","source":{"keep":true}}"#
    );
}

#[test]
fn json_patch_supports_root_replacement_and_array_append() {
    assert_eq!(
        json_value(
            r#"RETURN json_patch(
                json('{"a":1}'),
                json('[{"op":"replace","path":"","value":{"items":[1]}}]')
            ) AS value"#
        )
        .to_canonical_string(),
        r#"{"items":[1]}"#
    );
    assert_eq!(
        json_value(
            r#"RETURN json_patch(
                json('{"items":[1]}'),
                json('[{"op":"add","path":"/items/-","value":2}]')
            ) AS value"#
        )
        .to_canonical_string(),
        r#"{"items":[1,2]}"#
    );
}

#[test]
fn json_patch_propagates_sql_null_arguments() {
    assert_eq!(
        single_value("RETURN json_patch(NULL, json('[]')) AS value", "value"),
        Value::Null
    );
    assert_eq!(
        single_value("RETURN json_patch(json('{}'), NULL) AS value", "value"),
        Value::Null
    );
}

#[test]
fn json_patch_reports_data_exception_for_invalid_patch_documents() {
    for source in [
        r#"RETURN json_patch(json('{"a":1}'), json('{}')) AS value"#,
        r#"RETURN json_patch(json('{"a":1}'), json('[{"op":"add","path":"/b"}]')) AS value"#,
        r#"RETURN json_patch(json('{"a":1}'), json('[{"op":"remove","path":"/missing"}]')) AS value"#,
        r#"RETURN json_patch(json('{"a":1}'), json('[{"op":"test","path":"/a","value":2}]')) AS value"#,
        r#"RETURN json_patch(json('{"a":{"b":1}}'), json('[{"op":"move","from":"/a","path":"/a/b/c"}]')) AS value"#,
        r#"RETURN json_patch(json('{"items":[1]}'), json('[{"op":"add","path":"/items/01","value":2}]')) AS value"#,
        r#"RETURN json_patch(json('{"a":1}'), json('[{"op":"add","path":"/bad~2escape","value":2}]')) AS value"#,
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn json_patch_composes_with_json_property_updates() {
    let graph = empty_closed_graph(14_005);
    let mut session = Session::new(&graph);
    session
        .execute_source(
            "CREATE NODE TYPE :Thing (payload :: JSON NOT NULL)",
            &EmptyProcedureRegistry,
        )
        .expect("JSON node type creates");
    session
        .execute_source(
            r#"INSERT (:Thing {payload: json('{"name":"alpha","tags":["agent"],"meta":{"score":1}}')})"#,
            &EmptyProcedureRegistry,
        )
        .expect("JSON node inserts");

    let graph_type = graph.graph_type().expect("graph has type");
    assert_eq!(
        graph_type.node_types[0].properties[0].value_type,
        PropertyValueType::Json
    );

    session
        .execute_source(
            r#"MATCH (n:Thing)
               SET n.payload = json_patch(
                   n.payload,
                   json('[
                       {"op":"replace","path":"/name","value":"beta"},
                       {"op":"add","path":"/tags/-","value":"graph"},
                       {"op":"remove","path":"/meta/score"},
                       {"op":"add","path":"/meta/current","value":true}
                   ]')
               )
               FINISH"#,
            &EmptyProcedureRegistry,
        )
        .expect("JSON property patch updates");

    let output = session
        .execute_source(
            "MATCH (n:Thing) RETURN json_get_text(n.payload, 'name') AS name, json_get_path_text(n.payload, 'tags', 1) AS tag, json_has_path(n.payload, 'meta', 'score') AS has_score, json_get_path_text(n.payload, 'meta', 'current') AS current",
            &EmptyProcedureRegistry,
        )
        .expect("patched JSON property reads");
    let table = rows_from_output(output);
    assert_eq!(
        column_values(&table, "name"),
        vec![Value::String(db_string("beta"))]
    );
    assert_eq!(
        column_values(&table, "tag"),
        vec![Value::String(db_string("graph"))]
    );
    assert_eq!(column_values(&table, "has_score"), vec![Value::Bool(false)]);
    assert_eq!(
        column_values(&table, "current"),
        vec![Value::String(db_string("true"))]
    );
}
