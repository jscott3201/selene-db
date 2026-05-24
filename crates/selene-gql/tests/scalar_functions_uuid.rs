//! BRIEF-135c implementation-defined UUID scalar function coverage.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, execute_read, execute_read_result};
use selene_core::{Value, feature_register::FeatureId};
use selene_gql::{feature_walk, parse};

const UUID_TEXT: &str = "018f1b6d-7b89-7cc0-9f40-2c6f8d4df101";

fn single_value(source: &str, column: &str) -> Value {
    let table = execute_read(source);
    let mut values = column_values(&table, column);
    assert_eq!(values.len(), 1);
    values.pop().expect("one row")
}

fn uuid_value(source: &str) -> uuid::Uuid {
    match single_value(source, "value") {
        Value::Uuid(value) => value,
        other => panic!("expected UUID value, got {other:?}"),
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
        observed.contains(&FeatureId::IM_UUID),
        "{source} should record IM_UUID, observed {observed:?}"
    );
}

#[test]
fn uuid_v4_returns_random_version_uuid() {
    let value = uuid_value("RETURN uuid_v4() AS value");
    assert_eq!(value.get_version(), Some(uuid::Version::Random));
}

#[test]
fn uuid_v7_returns_sortable_random_version_uuid() {
    let value = uuid_value("RETURN uuid_v7() AS value");
    assert_eq!(value.get_version(), Some(uuid::Version::SortRand));
}

#[test]
fn uuid_function_parses_hyphenated_string() {
    assert_eq!(
        uuid_value(&format!("RETURN uuid('{UUID_TEXT}') AS value")),
        uuid::Uuid::parse_str(UUID_TEXT).expect("test UUID parses")
    );
}

#[test]
fn uuid_function_propagates_null() {
    assert_eq!(
        single_value("RETURN uuid(null) AS value", "value"),
        Value::Null
    );
}

#[test]
fn uuid_function_rejects_invalid_and_non_string_arguments() {
    assert_status("RETURN uuid('not-a-uuid') AS value", "22G03");
    assert_status("RETURN uuid(7) AS value", "22G03");
}

#[test]
fn uuid_functions_reject_wrong_arity() {
    for source in [
        "RETURN uuid_v4(1) AS value",
        "RETURN uuid_v7(1) AS value",
        "RETURN uuid() AS value",
        "RETURN uuid('a', 'b') AS value",
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn uuid_functions_are_flagged_as_implementation_defined() {
    for source in [
        "RETURN uuid_v4() AS value",
        "RETURN uuid_v7() AS value",
        "RETURN uuid('018f1b6d-7b89-7cc0-9f40-2c6f8d4df101') AS value",
    ] {
        assert_feature_recorded(source);
    }
}

#[test]
fn uuid_literals_and_type_names_are_flagged_as_implementation_defined() {
    for source in [
        "RETURN UUID '018f1b6d-7b89-7cc0-9f40-2c6f8d4df101' AS value",
        "RETURN null IS TYPED UUID AS value",
    ] {
        assert_feature_recorded(source);
    }
}
