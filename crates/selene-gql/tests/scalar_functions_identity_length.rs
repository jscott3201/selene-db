//! BRIEF-135b-2 ISO identity and length scalar function coverage.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, execute_read, execute_read_result};
use selene_core::{Value, feature_register::FeatureId};
use selene_gql::{feature_walk, parse};

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

fn assert_external_id(value: Value, prefix: &str) {
    let Value::ExternalString(actual) = value else {
        panic!("expected ExternalString ID, got {value:?}");
    };
    assert!(
        actual.starts_with(prefix),
        "expected {prefix} ID, got {actual}"
    );
}

#[test]
fn element_id_returns_external_string_for_nodes_and_edges() {
    assert_external_id(
        single_value("MATCH (n:Person) RETURN element_id(n) AS id LIMIT 1", "id"),
        "NodeId(",
    );
    assert_external_id(
        single_value("MATCH ()-[e]->() RETURN element_id(e) AS id LIMIT 1", "id"),
        "EdgeId(",
    );
}

#[test]
fn element_id_propagates_null() {
    assert_eq!(
        single_value("RETURN element_id(null) AS id", "id"),
        Value::Null
    );
}

#[test]
fn element_id_rejects_non_element_arguments() {
    for source in [
        "RETURN element_id(1) AS id",
        "RETURN element_id('x') AS id",
        "MATCH (n:Person) RETURN element_id([n]) AS id LIMIT 1",
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn element_id_rejects_wrong_arity() {
    assert_status("RETURN element_id() AS id", "22G03");
    assert_status("RETURN element_id(1, 2) AS id", "22G03");
}

#[test]
fn element_id_is_stable_for_equality_within_statement() {
    assert_eq!(
        single_value(
            "MATCH (n:Person) RETURN element_id(n) = element_id(n) AS same LIMIT 1",
            "same",
        ),
        Value::Bool(true)
    );
}

#[test]
fn element_id_records_g100_feature() {
    let statement = parse("MATCH (n) RETURN element_id(n) AS id").expect("source parses");
    let features = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    assert!(
        features.contains(&FeatureId::G100),
        "element_id should record G100, observed {features:?}"
    );
}
