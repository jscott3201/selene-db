//! List value function coverage for ISO/IEC 39075:2024 section 20.16.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, execute_read, execute_read_result};
use selene_core::{Value, feature_register::FeatureId};
use selene_gql::{feature_walk, parse};

fn single_value(source: &str, column: &str) -> Value {
    let table = execute_read(source);
    let mut values = column_values(&table, column);
    assert_eq!(values.len(), 1, "{source}");
    values.pop().expect("one row")
}

fn assert_status(source: &str, expected: &str) {
    let err = execute_read_result(source).expect_err("query should fail");
    assert_eq!(err.gqlstatus().as_str(), expected, "source: {source}");
}

#[test]
fn trim_list_function_removes_tail_elements() {
    assert_eq!(
        single_value("RETURN trim([1, 2, 3, 4], 2) AS value", "value"),
        Value::List(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        single_value("RETURN trim([1, 2], 0) AS value", "value"),
        Value::List(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        single_value("RETURN trim([1, 2], 2) AS value", "value"),
        Value::List(vec![])
    );
}

#[test]
fn trim_list_function_propagates_nulls_in_iso_evaluation_order() {
    assert_eq!(
        single_value("RETURN trim(1 / 0, null) AS value", "value"),
        Value::Null
    );
    assert_eq!(
        single_value("RETURN trim(null, 0) AS value", "value"),
        Value::Null
    );
}

#[test]
fn trim_list_function_reports_list_element_errors() {
    assert_status("RETURN trim([1], -1) AS value", "22G0C");
    assert_status("RETURN trim([1], 2) AS value", "22G0C");
}

#[test]
fn trim_list_function_rejects_non_list_or_non_integer_arguments() {
    assert_status("RETURN trim('abc', 1) AS value", "22G03");
    assert_status("RETURN trim([1], 1.5) AS value", "22G03");
}

#[test]
fn trim_list_function_records_gv50() {
    let statement = parse("MATCH (n) RETURN trim(n.values, 1)").expect("source parses");
    let features = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    assert!(
        features.contains(&FeatureId::GV50),
        "trim list function should record GV50, observed {features:?}"
    );
}
