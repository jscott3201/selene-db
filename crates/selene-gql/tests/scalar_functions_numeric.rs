//! BRIEF-135b ISO §20.22 numeric scalar function coverage.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, db_string, execute_read, execute_read_result};
use selene_core::{GraphId, Value, feature_register::FeatureId};
use selene_gql::{EmptyProcedureRegistry, Session, feature_walk, parse};
use selene_graph::SharedGraph;

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
fn scalar_functions_numeric_gf01_enhanced_numeric_functions_return_expected_values() {
    let cases = [
        ("RETURN abs(-3) AS value", Value::Int(3)),
        ("RETURN mod(7, 4) AS value", Value::Int(3)),
        ("RETURN floor(1.8) AS value", Value::Float(1.0)),
        ("RETURN ceil(1.2) AS value", Value::Float(2.0)),
        ("RETURN ceiling(1.2) AS value", Value::Float(2.0)),
        ("RETURN sqrt(9) AS value", Value::Float(3.0)),
    ];

    for (source, expected) in cases {
        assert_eq!(single_value(source, "value"), expected, "source: {source}");
    }
}

#[test]
fn scalar_functions_numeric_gf01_enhanced_numeric_functions_propagate_null() {
    for source in [
        "RETURN abs(null) AS value",
        "RETURN mod(null, 4) AS value",
        "RETURN mod(7, null) AS value",
        "RETURN floor(null) AS value",
        "RETURN ceil(null) AS value",
        "RETURN ceiling(null) AS value",
        "RETURN sqrt(null) AS value",
    ] {
        assert_eq!(
            single_value(source, "value"),
            Value::Null,
            "source: {source}"
        );
    }
}

#[test]
fn scalar_functions_numeric_gf01_enhanced_numeric_functions_reject_non_numeric_arguments() {
    for source in [
        "RETURN abs('x') AS value",
        "RETURN mod('x', 4) AS value",
        "RETURN mod(7, 'x') AS value",
        "RETURN floor('x') AS value",
        "RETURN ceil('x') AS value",
        "RETURN ceiling('x') AS value",
        "RETURN sqrt('x') AS value",
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn scalar_functions_numeric_gf01_enhanced_numeric_domain_errors_use_iso_statuses() {
    assert_status("RETURN sqrt(-1) AS value", "22003");
    assert_status("RETURN mod(7, 0) AS value", "22012");
}

#[test]
fn scalar_functions_numeric_round_is_not_in_the_iso_numeric_function_set() {
    let err = execute_read_result("RETURN round(1.6) AS value")
        .expect_err("round is not in the closed scalar-function set");
    assert!(matches!(
        &err,
        selene_gql::ExecutorError::UnknownFunction { name, .. } if name == "round"
    ));
    assert_eq!(err.gqlstatus().as_str(), "22G03");
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

#[test]
fn scalar_functions_numeric_gf03_logarithmic_functions_return_expected_values() {
    let cases = [
        ("RETURN ln(1) AS value", 0.0),
        ("RETURN ln(2.718281828459045) AS value", 1.0),
        ("RETURN log(2, 8) AS value", 3.0),
        ("RETURN log(10, 100) AS value", 2.0),
        ("RETURN log10(100) AS value", 2.0),
        ("RETURN exp(0) AS value", 1.0),
        ("RETURN exp(1) AS value", std::f64::consts::E),
    ];

    for (source, expected) in cases {
        assert_float_near(single_value(source, "value"), expected);
    }
}

#[test]
fn scalar_functions_numeric_gf03_logarithmic_functions_propagate_null() {
    for source in [
        "RETURN ln(null) AS value",
        "RETURN log(null, 8) AS value",
        "RETURN log(2, null) AS value",
        "RETURN log10(null) AS value",
        "RETURN exp(null) AS value",
        "RETURN power(null, 2) AS value",
        "RETURN power(2, null) AS value",
    ] {
        assert_eq!(
            single_value(source, "value"),
            Value::Null,
            "source: {source}"
        );
    }
}

#[test]
fn scalar_functions_numeric_gf03_logarithmic_functions_reject_non_numeric_arguments() {
    for source in [
        "RETURN ln('x') AS value",
        "RETURN log('x', 8) AS value",
        "RETURN log(2, 'x') AS value",
        "RETURN log10('x') AS value",
        "RETURN exp('x') AS value",
        "RETURN power('x', 2) AS value",
        "RETURN power(2, 'x') AS value",
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn scalar_functions_numeric_gf03_logarithmic_domain_errors_use_iso_statuses() {
    assert_status("RETURN ln(0) AS value", "2201E");
    assert_status("RETURN ln(-1) AS value", "2201E");
    assert_status("RETURN log(1, 10) AS value", "22003");
    assert_status("RETURN log(0, 10) AS value", "22003");
    assert_status("RETURN log(10, 0) AS value", "22003");
    assert_status("RETURN log10(0) AS value", "22003");
    assert_status("RETURN exp(1000) AS value", "22003");
}

#[test]
fn scalar_functions_numeric_gf03_logarithmic_sanity_round_trips() {
    assert_float_near(single_value("RETURN exp(ln(2.5)) AS value", "value"), 2.5);
    assert_float_near(
        single_value("RETURN log10(power(10, 5)) AS value", "value"),
        5.0,
    );
}

#[test]
fn scalar_functions_numeric_gf03_logarithmic_function_flags_are_recorded() {
    for source in [
        "RETURN ln(1) AS value",
        "RETURN log(2, 8) AS value",
        "RETURN log10(100) AS value",
        "RETURN exp(1) AS value",
        "RETURN power(2, 3) AS value",
    ] {
        let statement = parse(source).expect(source);
        let features = feature_walk(&statement)
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            features.contains(&FeatureId::GF03),
            "{source} should record GF03, observed {features:?}"
        );
    }
}

#[test]
fn scalar_functions_numeric_power_gr11_boundary_cases() {
    let cases = [
        ("RETURN power(0.0, 0.0) AS value", 1.0),
        ("RETURN power(0.0, 2.0) AS value", 0.0),
        ("RETURN power(-2.0, 2.0) AS value", 4.0),
        ("RETURN power(-2.0, 3.0) AS value", -8.0),
    ];

    for (source, expected) in cases {
        assert_float_near(single_value(source, "value"), expected);
    }
}

#[test]
fn scalar_functions_numeric_power_gr11_invalid_argument_cases_use_2201f() {
    assert_status("RETURN power(0.0, -1.0) AS value", "2201F");
    assert_status("RETURN power(-2.0, 1.5) AS value", "2201F");
}

#[test]
fn scalar_functions_numeric_power_gr11_overflow_uses_22003() {
    assert_status("RETURN power(2.0, 1024.0) AS value", "22003");
    assert_status("RETURN power(10.0, 400.0) AS value", "22003");
}

#[test]
fn scalar_functions_numeric_power_zero_base_rejects_nan_exponent() {
    let graph = SharedGraph::new(GraphId::new(13_521));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("nan"), Value::Float(f64::NAN));

    let err = session
        .execute_source("RETURN power(0, $nan) AS value", &EmptyProcedureRegistry)
        .expect_err("NaN exponent should reject");

    assert_eq!(err.gqlstatus().as_str(), "22003");
}
