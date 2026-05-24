//! BRIEF-135b ISO §20.22 numeric scalar function coverage.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, execute_read, execute_read_result};
use selene_core::{Value, feature_register::FeatureId};
use selene_gql::{feature_walk, parse};

const EPSILON: f64 = 1e-12;
const PI: f64 = std::f64::consts::PI;

fn single_value(source: &str, column: &str) -> Value {
    let table = execute_read(source);
    let mut values = column_values(&table, column);
    assert_eq!(values.len(), 1);
    values.pop().expect("one row")
}

fn assert_float_near(value: Value, expected: f64) {
    match value {
        Value::Float(actual) => assert!(
            (actual - expected).abs() < EPSILON,
            "expected {expected}, got {actual}"
        ),
        other => panic!("expected float {expected}, got {other:?}"),
    }
}

fn assert_status(source: &str, expected: &str) {
    let err = execute_read_result(source).expect_err("query should fail");
    assert_eq!(err.gqlstatus().as_str(), expected, "source: {source}");
}

#[test]
fn scalar_functions_numeric_gf02_trigonometric_functions_return_expected_values() {
    let cases = [
        ("RETURN sin(0) AS value", 0.0),
        ("RETURN cos(0) AS value", 1.0),
        ("RETURN tan(0) AS value", 0.0),
        ("RETURN sinh(0) AS value", 0.0),
        ("RETURN cosh(0) AS value", 1.0),
        ("RETURN tanh(0) AS value", 0.0),
        ("RETURN asin(0) AS value", 0.0),
        ("RETURN acos(1) AS value", 0.0),
        ("RETURN atan(0) AS value", 0.0),
        ("RETURN degrees(3.141592653589793) AS value", 180.0),
        ("RETURN radians(180) AS value", PI),
    ];

    for (source, expected) in cases {
        assert_float_near(single_value(source, "value"), expected);
    }
}

#[test]
fn scalar_functions_numeric_cotangent_returns_reciprocal_tangent_for_non_zero_divisor() {
    assert_float_near(
        single_value("RETURN cot(0.7853981633974483) AS value", "value"),
        1.0,
    );
}

#[test]
fn scalar_functions_numeric_gf02_trigonometric_functions_propagate_null() {
    for name in [
        "sin", "cos", "tan", "cot", "sinh", "cosh", "tanh", "asin", "acos", "atan", "degrees",
        "radians",
    ] {
        let source = format!("RETURN {name}(null) AS value");
        assert_eq!(
            single_value(&source, "value"),
            Value::Null,
            "source: {source}"
        );
    }
}

#[test]
fn scalar_functions_numeric_gf02_trigonometric_functions_reject_non_numeric_arguments() {
    for name in [
        "sin", "cos", "tan", "cot", "sinh", "cosh", "tanh", "asin", "acos", "atan", "degrees",
        "radians",
    ] {
        let source = format!("RETURN {name}('x') AS value");
        assert_status(&source, "22G03");
    }
}

#[test]
fn scalar_functions_numeric_gf02_trigonometric_domain_errors_use_22003() {
    assert_status("RETURN asin(2) AS value", "22003");
    assert_status("RETURN acos(-2) AS value", "22003");
    assert_status("RETURN cot(0) AS value", "22003");
}

#[test]
fn scalar_functions_numeric_gf02_inverse_trigonometric_boundaries_are_inclusive() {
    assert_float_near(single_value("RETURN asin(1) AS value", "value"), PI / 2.0);
    assert_float_near(single_value("RETURN asin(-1) AS value", "value"), -PI / 2.0);
    assert_float_near(single_value("RETURN acos(-1) AS value", "value"), PI);
}

#[test]
fn scalar_functions_numeric_gf02_trigonometric_function_flags_are_recorded() {
    for name in [
        "sin", "cos", "tan", "cot", "sinh", "cosh", "tanh", "asin", "acos", "atan", "degrees",
        "radians",
    ] {
        let argument = if name == "cot" { "1" } else { "0" };
        let source = format!("RETURN {name}({argument}) AS value");
        let statement = parse(&source).expect(&source);
        let features = feature_walk(&statement)
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            features.contains(&FeatureId::GF02),
            "{source} should record GF02, observed {features:?}"
        );
    }
}
