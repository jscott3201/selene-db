//! BRIEF-135d ISO string scalar function coverage.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, execute_read, execute_read_result};
use selene_core::{Value, feature_register::FeatureId};
use selene_gql::{EmptyProcedureRegistry, analyze, feature_walk, parse};

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

fn assert_status(source: &str, expected: &str) {
    let err = execute_read_result(source).expect_err("query should fail");
    assert_eq!(err.gqlstatus().as_str(), expected, "source: {source}");
}

fn external_string(value: Value) -> String {
    let Value::ExternalString(value) = value else {
        panic!("expected ExternalString, got {value:?}");
    };
    value.to_string()
}

fn assert_feature_recorded(source: &str, expected: FeatureId) {
    let statement = parse(source).expect(source);
    let observed = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();
    assert!(
        observed.contains(&expected),
        "{source} should record {expected:?}, observed {observed:?}"
    );
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

#[test]
fn left_and_right_return_unicode_prefixes_and_suffixes() {
    let cases = [
        ("RETURN left('café', 3) AS value", "caf"),
        ("RETURN right('café', 2) AS value", "fé"),
        ("RETURN left('日本語', 2) AS value", "日本"),
        ("RETURN right('日本語', 99) AS value", "日本語"),
    ];
    for (source, expected) in cases {
        assert_eq!(external_string(single_value(source, "value")), expected);
    }
}

#[test]
fn left_and_right_propagate_nulls_and_reject_bad_lengths() {
    for source in [
        "RETURN left(null, 1) AS value",
        "RETURN left('abc', null) AS value",
        "RETURN right(null, 1) AS value",
        "RETURN right('abc', null) AS value",
    ] {
        assert_eq!(
            single_value(source, "value"),
            Value::Null,
            "source: {source}"
        );
    }
    assert_status("RETURN left('abc', -1) AS value", "22011");
    assert_status("RETURN right('abc', -1) AS value", "22011");
    assert_status("RETURN left('abc', 'x') AS value", "22G03");
    assert_status("RETURN right(7, 1) AS value", "22G03");
}

#[test]
fn multi_character_trim_family_trims_default_and_custom_character_sets() {
    let cases = [
        ("RETURN btrim('  hello  ') AS value", "hello"),
        ("RETURN ltrim('  hello  ') AS value", "hello  "),
        ("RETURN rtrim('  hello  ') AS value", "  hello"),
        ("RETURN btrim('xyhello yx', 'xy ') AS value", "hello"),
        ("RETURN ltrim('xyhello', 'xy') AS value", "hello"),
        ("RETURN rtrim('helloxy', 'xy') AS value", "hello"),
    ];
    for (source, expected) in cases {
        assert_eq!(external_string(single_value(source, "value")), expected);
    }
}

#[test]
fn multi_character_trim_family_propagates_nulls_and_rejects_non_strings() {
    for source in [
        "RETURN btrim(null) AS value",
        "RETURN btrim('abc', null) AS value",
        "RETURN ltrim(null, 'x') AS value",
        "RETURN rtrim('abc', null) AS value",
    ] {
        assert_eq!(
            single_value(source, "value"),
            Value::Null,
            "source: {source}"
        );
    }
    assert_status("RETURN btrim(7) AS value", "22G03");
    assert_status("RETURN ltrim('abc', 7) AS value", "22G03");
}

#[test]
fn multi_character_trim_family_records_gf05() {
    for source in [
        "RETURN btrim('x') AS value",
        "RETURN ltrim('x') AS value",
        "RETURN rtrim('x') AS value",
    ] {
        assert_feature_recorded(source, FeatureId::GF05);
    }
}
