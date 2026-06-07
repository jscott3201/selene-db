//! JSON scalar selector-array path document coverage.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, execute_read, execute_read_result};
use selene_core::Value;

fn single_value(source: &str, column: &str) -> Value {
    let table = execute_read(source);
    let mut values = column_values(&table, column);
    assert_eq!(values.len(), 1);
    values.pop().expect("one row")
}

fn assert_status(source: &str, expected: &str) {
    let err = execute_read_result(source).expect_err("query should fail");
    assert_eq!(err.gqlstatus().as_str(), expected, "source: {source}");
}

#[test]
fn json_path_functions_accept_selector_array_documents() {
    match single_value(
        r#"RETURN json_get_path(
             json('{"memory":{"events":[{"score":7},{"score":9}],"current":true}}'),
             json_array('memory', 'events', 1, 'score')
           ) AS value"#,
        "value",
    ) {
        Value::Json(value) => assert_eq!(value.to_canonical_string(), "9"),
        other => panic!("expected JSON value, got {other:?}"),
    }

    assert_eq!(
        single_value(
            r#"RETURN json_get_path_text(
                 json('{"events":[{"kind":"semantic"},{"kind":"episodic"}]}'),
                 json_array('events', -1, 'kind')
               ) AS value"#,
            "value",
        ),
        Value::String(selene_core::db_string("episodic").unwrap())
    );

    assert_eq!(
        single_value(
            r#"RETURN json_get_path_scalar(
                 json('{"memory":{"current":true}}'),
                 json_array('memory', 'current')
               ) AS value"#,
            "value",
        ),
        Value::Bool(true)
    );

    assert_eq!(
        single_value(
            r#"RETURN json_has_path(
                 json('{"memory":{"score":null}}'),
                 json_array('memory', 'score')
               ) AS value"#,
            "value",
        ),
        Value::Bool(true)
    );
}

#[test]
fn json_path_documents_preserve_missing_and_sql_null_semantics() {
    assert_eq!(
        single_value(
            r#"RETURN json_get_path(json('{"a":{"b":1}}'), json_array('a', 'missing')) AS value"#,
            "value",
        ),
        Value::Null
    );
    assert_eq!(
        single_value(
            r#"RETURN json_has_path(json('{"a":{"b":1}}'), json_array('a', 'missing')) AS value"#,
            "value",
        ),
        Value::Bool(false)
    );
    assert_eq!(
        single_value(
            r#"RETURN json_get_path(NULL, json_array('a')) AS value"#,
            "value",
        ),
        Value::Null
    );
    assert_eq!(
        single_value(
            r#"RETURN json_has_path(json('{"a":1}'), NULL) AS value"#,
            "value",
        ),
        Value::Null
    );
}

#[test]
fn json_path_documents_report_data_exceptions_for_bad_documents() {
    let selectors = (0..65).map(|_| r#""a""#).collect::<Vec<_>>().join(",");
    let too_deep = format!("RETURN json_get_path(json('{{}}'), json('[{selectors}]')) AS value");
    for source in [
        r#"RETURN json_get_path(json('{}'), json_object('a', 1)) AS value"#,
        r#"RETURN json_get_path(json('{}'), json_array()) AS value"#,
        r#"RETURN json_get_path(json('{"a":1}'), json_array(TRUE)) AS value"#,
        r#"RETURN json_get_path(json('[1,2]'), json_array(1.5)) AS value"#,
        r#"RETURN json_get_path(json('[1,2]'), json_array('bad')) AS value"#,
        too_deep.as_str(),
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn shallow_json_get_does_not_accept_path_documents() {
    assert_status(
        r#"RETURN json_get(json('{"a":{"b":1}}'), json_array('a', 'b')) AS value"#,
        "22G03",
    );
}
