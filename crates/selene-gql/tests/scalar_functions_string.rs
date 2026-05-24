//! BRIEF-135d ISO string scalar function coverage.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, execute_read};
use selene_core::Value;
use selene_gql::{EmptyProcedureRegistry, analyze, parse};

fn single_value(source: &str, column: &str) -> Value {
    let table = execute_read(source);
    let mut values = column_values(&table, column);
    assert_eq!(values.len(), 1);
    values.pop().expect("one row")
}

fn assert_analysis_status(source: &str, expected: &str) {
    let statement = parse(source).expect("source parses");
    let err = analyze(statement, &EmptyProcedureRegistry, None).expect_err("query should fail");
    assert_eq!(err.gqlstatus().as_str(), expected, "source: {source}");
}

fn external_string(value: Value) -> String {
    let Value::ExternalString(value) = value else {
        panic!("expected ExternalString, got {value:?}");
    };
    value.to_string()
}

#[test]
fn normalize_defaults_to_nfc_and_returns_external_string() {
    assert_eq!(
        external_string(single_value(
            "RETURN NORMALIZE('e\u{301}') AS value",
            "value"
        )),
        "\u{00e9}"
    );
}

#[test]
fn normalize_accepts_all_unicode_normal_forms() {
    let cases = [
        ("NFC", "\u{00e9}"),
        ("NFD", "e\u{301}"),
        ("NFKC", "1"),
        ("NFKD", "1"),
    ];
    for (form, expected) in cases {
        let source = if form.starts_with("NFK") {
            format!("RETURN NORMALIZE('\u{2460}', {form}) AS value")
        } else {
            format!("RETURN NORMALIZE('e\u{301}', {form}) AS value")
        };
        assert_eq!(external_string(single_value(&source, "value")), expected);
    }
}

#[test]
fn normalize_propagates_null_and_rejects_non_string_source() {
    assert_eq!(
        single_value("RETURN NORMALIZE(null) AS value", "value"),
        Value::Null
    );
    assert_analysis_status("RETURN NORMALIZE(7) AS value", "22G03");
}

#[test]
fn is_normalized_evaluates_forms_and_preserves_unknown() {
    let cases = [
        (
            "RETURN 'e\u{301}' IS NFD NORMALIZED AS value",
            Value::Bool(true),
        ),
        (
            "RETURN 'e\u{301}' IS NFC NORMALIZED AS value",
            Value::Bool(false),
        ),
        (
            "RETURN '\u{00e9}' IS NORMALIZED AS value",
            Value::Bool(true),
        ),
        (
            "RETURN '\u{00e9}' IS NOT NFD NORMALIZED AS value",
            Value::Bool(true),
        ),
        ("RETURN null IS NORMALIZED AS value", Value::Null),
    ];
    for (source, expected) in cases {
        assert_eq!(single_value(source, "value"), expected, "source: {source}");
    }
}

#[test]
fn is_normalized_rejects_non_string_operand() {
    assert_analysis_status("RETURN 7 IS NORMALIZED AS value", "22G03");
}
