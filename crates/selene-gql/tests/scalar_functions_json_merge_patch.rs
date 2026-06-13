//! Focused coverage for RFC 7396 JSON merge-patch helpers.

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

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: db_string("json.merge_patch.graph"),
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
fn json_merge_patch_applies_rfc7396_object_update_semantics() {
    let value = json_value(
        r#"RETURN json_merge_patch(
            json('{"title":"Goodbye!","author":{"givenName":"John","familyName":"Doe"},"tags":["example","sample"],"content":"unchanged"}'),
            json('{"title":"Hello!","phoneNumber":"+01-123-456-7890","author":{"familyName":null},"tags":["example"]}')
        ) AS value"#,
    );
    assert_eq!(
        value.to_canonical_string(),
        r#"{"author":{"givenName":"John"},"content":"unchanged","phoneNumber":"+01-123-456-7890","tags":["example"],"title":"Hello!"}"#
    );

    assert_eq!(
        json_value(r#"RETURN json_merge_patch(json('{"a":"foo"}'), json('null')) AS value"#)
            .to_canonical_string(),
        "null"
    );
    assert_eq!(
        json_value(r#"RETURN json_merge_patch(json('{"a":"b"}'), json('["c"]')) AS value"#)
            .to_canonical_string(),
        r#"["c"]"#
    );
}

#[test]
fn json_merge_patch_propagates_sql_null_arguments() {
    assert_eq!(
        single_value(
            "RETURN json_merge_patch(NULL, json('{}')) AS value",
            "value"
        ),
        Value::Null
    );
    assert_eq!(
        single_value(
            "RETURN json_merge_patch(json('{}'), NULL) AS value",
            "value"
        ),
        Value::Null
    );
}

#[test]
fn json_merge_patch_composes_with_json_property_updates() {
    let graph = empty_closed_graph(14_004);
    let mut session = Session::new(&graph);
    session
        .execute_source(
            "CREATE NODE TYPE :Thing (payload :: JSON NOT NULL)",
            &EmptyProcedureRegistry,
        )
        .expect("JSON node type creates");
    session
        .execute_source(
            r#"INSERT (:Thing {payload: json('{"name":"beta","meta":{"seen":true}}')})"#,
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
               SET n.payload = json_merge_patch(
                   n.payload,
                   json('{"name":null,"meta":{"seen":false,"source":"merge"}}')
               )
               FINISH"#,
            &EmptyProcedureRegistry,
        )
        .expect("JSON property merge-patch updates");

    let output = session
        .execute_source(
            "MATCH (n:Thing) RETURN json_has_path(n.payload, 'name') AS has_name, json_get_path_text(n.payload, 'meta', 'seen') AS seen, json_get_path_text(n.payload, 'meta', 'source') AS source",
            &EmptyProcedureRegistry,
        )
        .expect("merge-patched JSON property reads");
    let table = rows_from_output(output);
    assert_eq!(column_values(&table, "has_name"), vec![Value::Bool(false)]);
    assert_eq!(
        column_values(&table, "seen"),
        vec![Value::String(db_string("false"))]
    );
    assert_eq!(
        column_values(&table, "source"),
        vec![Value::String(db_string("merge"))]
    );
}
