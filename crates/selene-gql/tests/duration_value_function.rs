//! Duration value function coverage for ISO/IEC 39075:2024 sections 20.28 and
//! 20.29.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, db_string, execute_read, execute_read_result};
use selene_core::Value;
use selene_gql::{
    Binding, BindingTableSchema, GqlStatus, Literal, NonEmpty, SourceSpan, ValueExpr,
};

fn span() -> SourceSpan {
    SourceSpan::new(0, 1)
}

fn string_lit(value: &str) -> ValueExpr {
    ValueExpr::Literal(Literal::String(db_string(value), span()))
}

fn null_lit() -> ValueExpr {
    ValueExpr::Literal(Literal::Null(span()))
}

fn bool_lit(value: bool) -> ValueExpr {
    ValueExpr::Literal(Literal::Bool(value, span()))
}

fn function_call(name: &str, args: Vec<ValueExpr>) -> ValueExpr {
    ValueExpr::FunctionCall {
        name: NonEmpty::try_from_vec(vec![db_string(name)]).expect("non-empty"),
        args,
        star: false,
        distinct: false,
        span: span(),
    }
}

fn eval(expr: &ValueExpr) -> Result<Value, selene_gql::ExecutorError> {
    let caps = selene_gql::ImplDefinedCaps::default();
    let ctx = exec_common::empty_graph_context(&caps);
    selene_gql::runtime::evaluate_for_test(
        expr,
        &Binding::empty(),
        &BindingTableSchema { columns: vec![] },
        &ctx,
    )
}

fn single_value(source: &str, column: &str) -> Value {
    let table = execute_read(source);
    let mut values = column_values(&table, column);
    assert_eq!(values.len(), 1, "{source}");
    values.pop().expect("one row")
}

fn status_for(source: &str) -> GqlStatus {
    execute_read_result(source)
        .expect_err("statement errors")
        .gqlstatus()
}

fn analysis_status_for(source: &str) -> GqlStatus {
    let statement = selene_gql::parse(source).expect("test source parses");
    selene_gql::analyze(statement, &selene_gql::EmptyProcedureRegistry, None)
        .expect_err("test source fails analysis")
        .gqlstatus()
}

#[test]
fn duration_function_parses_string_and_null_parameters() {
    assert_eq!(
        eval(&function_call("duration", vec![string_lit("P2M")])).unwrap(),
        Value::Duration(Box::new("P2M".parse().unwrap()))
    );
    assert_eq!(
        eval(&function_call("duration", vec![null_lit()])).unwrap(),
        Value::Null
    );

    let err = eval(&function_call("duration", vec![bool_lit(true)]))
        .expect_err("duration rejects non-string/non-record parameters");
    assert_eq!(err.gqlstatus().as_str(), "22G03");

    let err = eval(&function_call("duration", vec![string_lit("not-duration")]))
        .expect_err("duration rejects invalid duration text");
    assert_eq!(err.gqlstatus(), GqlStatus::INVALID_DURATION_FORMAT);
}

#[test]
fn duration_record_constructor_builds_year_month_and_day_time_values() {
    assert_eq!(
        single_value("RETURN DURATION({years: 1, months: 2}) AS value", "value"),
        Value::Duration(Box::new("P1Y2M".parse().unwrap()))
    );
    assert_eq!(
        single_value("RETURN DURATION({months: 3}) AS value", "value"),
        Value::Duration(Box::new("P0Y3M".parse().unwrap()))
    );
    assert_eq!(
        single_value(
            "RETURN DURATION({days: 3, hours: 4, minutes: 5, seconds: 6, milliseconds: 7}) AS value",
            "value"
        ),
        Value::Duration(Box::new("P3DT4H5M6.007S".parse().unwrap()))
    );
    assert_eq!(
        single_value(
            "RETURN DURATION({days: 0, seconds: 1, nanoseconds: 2}) AS value",
            "value"
        ),
        Value::Duration(Box::new("P0DT0H0M1.000000002S".parse().unwrap()))
    );
}

#[test]
fn duration_absolute_value_function_returns_non_negative_duration() {
    assert_eq!(
        single_value("RETURN ABS(DURATION('-P2M')) AS value", "value"),
        Value::Duration(Box::new("P2M".parse().unwrap()))
    );
    assert_eq!(
        single_value("RETURN ABS(DURATION('-PT1H2.003S')) AS value", "value"),
        Value::Duration(Box::new("PT1H2.003S".parse().unwrap()))
    );
    assert_eq!(
        single_value("RETURN ABS(DURATION('P3DT4H')) AS value", "value"),
        Value::Duration(Box::new("P3DT4H".parse().unwrap()))
    );
    assert_eq!(
        single_value("RETURN ABS(NULL) AS value", "value"),
        Value::Null
    );
}

#[test]
fn duration_between_returns_day_time_duration_for_temporal_instants() {
    assert_eq!(
        single_value(
            "RETURN DURATION_BETWEEN(DATE('2026-01-01'), DATE('2026-01-03')) AS value",
            "value"
        ),
        Value::Duration(Box::new("P2D".parse().unwrap()))
    );
    assert_eq!(
        single_value(
            "RETURN DURATION_BETWEEN(LOCAL_DATETIME('2026-01-01T00:00:00'), \
             LOCAL_DATETIME('2026-01-02T01:01:01.000000002')) AS value",
            "value"
        ),
        Value::Duration(Box::new("P1DT1H1M1.000000002S".parse().unwrap()))
    );
    assert_eq!(
        single_value(
            "RETURN DURATION_BETWEEN(LOCAL_TIME('12:00:00'), LOCAL_TIME('14:30:00')) AS value",
            "value"
        ),
        Value::Duration(Box::new("PT2H30M".parse().unwrap()))
    );
    assert_eq!(
        single_value(
            "RETURN DURATION_BETWEEN(ZONED_DATETIME('2026-01-01T00:00:00Z'), \
             ZONED_DATETIME('2026-01-01T01:00:00Z')) AS value",
            "value"
        ),
        Value::Duration(Box::new("PT1H".parse().unwrap()))
    );
    assert_eq!(
        single_value(
            "RETURN DURATION_BETWEEN(ZONED_TIME('12:00:00Z'), \
             ZONED_TIME('14:30:00Z')) AS value",
            "value"
        ),
        Value::Duration(Box::new("PT2H30M".parse().unwrap()))
    );
}

#[test]
fn duration_between_preserves_direction_and_null_semantics() {
    assert_eq!(
        single_value(
            "RETURN DURATION_BETWEEN(DATE('2026-01-03'), DATE('2026-01-01')) AS value",
            "value"
        ),
        Value::Duration(Box::new("-P2D".parse().unwrap()))
    );
    assert_eq!(
        single_value(
            "RETURN DURATION_BETWEEN(NULL, DATE('2026-01-01')) AS value",
            "value"
        ),
        Value::Null
    );
    assert_eq!(
        single_value(
            "RETURN DURATION_BETWEEN(DATE('2026-01-01'), NULL) AS value",
            "value"
        ),
        Value::Null
    );
}

#[test]
fn duration_addition_and_subtraction_normalize_same_unit_group_values() {
    assert_eq!(
        single_value(
            "RETURN DURATION('PT1H30M') + DURATION('PT45M') AS value",
            "value"
        ),
        Value::Duration(Box::new("PT2H15M".parse().unwrap()))
    );
    assert_eq!(
        single_value(
            "RETURN DURATION('PT2H') - DURATION('PT30M') AS value",
            "value"
        ),
        Value::Duration(Box::new("PT1H30M".parse().unwrap()))
    );
    assert_eq!(
        single_value(
            "RETURN DURATION('P1Y10M') + DURATION('P4M') AS value",
            "value"
        ),
        Value::Duration(Box::new("P2Y2M".parse().unwrap()))
    );
    assert_eq!(
        single_value("RETURN DURATION('P1M') - DURATION('P3M') AS value", "value"),
        Value::Duration(Box::new("-P2M".parse().unwrap()))
    );
}

#[test]
fn duration_arithmetic_preserves_unary_sign_and_null_semantics() {
    assert_eq!(
        single_value("RETURN -DURATION('PT1H') AS value", "value"),
        Value::Duration(Box::new("-PT1H".parse().unwrap()))
    );
    assert_eq!(
        single_value("RETURN DURATION('PT1H') + NULL AS value", "value"),
        Value::Null
    );
    assert_eq!(
        single_value("RETURN NULL - DURATION('PT1H') AS value", "value"),
        Value::Null
    );
    assert_eq!(single_value("RETURN -NULL AS value", "value"), Value::Null);
}

#[test]
fn duration_scaling_normalizes_same_unit_group_values() {
    assert_eq!(
        single_value("RETURN DURATION('PT1H30M') * 2 AS value", "value"),
        Value::Duration(Box::new("PT3H".parse().unwrap()))
    );
    assert_eq!(
        single_value("RETURN 2 * DURATION('PT45M') AS value", "value"),
        Value::Duration(Box::new("PT1H30M".parse().unwrap()))
    );
    assert_eq!(
        single_value("RETURN DURATION('PT3H') / 2 AS value", "value"),
        Value::Duration(Box::new("PT1H30M".parse().unwrap()))
    );
    assert_eq!(
        single_value("RETURN DURATION('PT1H') * 1.5 AS value", "value"),
        Value::Duration(Box::new("PT1H30M".parse().unwrap()))
    );
    assert_eq!(
        single_value(
            "RETURN DURATION('P2DT3H4M5.000000006S') * 1 AS value",
            "value"
        ),
        Value::Duration(Box::new("P2DT3H4M5.000000006S".parse().unwrap()))
    );
    assert_eq!(
        single_value("RETURN DURATION('P1Y') * 0.5 AS value", "value"),
        Value::Duration(Box::new("P6M".parse().unwrap()))
    );
}

#[test]
fn duration_scaling_preserves_null_semantics() {
    assert_eq!(
        single_value("RETURN DURATION('PT1H') * NULL AS value", "value"),
        Value::Null
    );
    assert_eq!(
        single_value("RETURN NULL * DURATION('PT1H') AS value", "value"),
        Value::Null
    );
    assert_eq!(
        single_value("RETURN DURATION('PT1H') / NULL AS value", "value"),
        Value::Null
    );
}

#[test]
fn duration_arithmetic_rejects_mismatched_unit_groups_and_duration_products() {
    assert_eq!(
        status_for("RETURN DURATION('P1M') + DURATION('PT1H') AS value").as_str(),
        "22G14"
    );
    assert_eq!(
        analysis_status_for("RETURN DURATION('PT1H') * DURATION('PT2H') AS value"),
        GqlStatus::DATATYPE_MISMATCH
    );
}

#[test]
fn duration_scaling_rejects_invalid_coefficients() {
    assert_eq!(
        status_for("RETURN DURATION('PT1H') / 0 AS value").as_str(),
        "22012"
    );
    assert_eq!(
        status_for("RETURN DURATION('P1M') / 2 AS value").as_str(),
        "22003"
    );
}

#[test]
fn duration_between_rejects_non_temporal_and_mixed_temporal_families() {
    assert_eq!(
        status_for("RETURN DURATION_BETWEEN(TRUE, DATE('2026-01-01')) AS value").as_str(),
        "22G03"
    );
    assert_eq!(
        status_for(
            "RETURN DURATION_BETWEEN(DATE('2026-01-01'), \
             LOCAL_DATETIME('2026-01-02T00:00:00')) AS value"
        )
        .as_str(),
        "22G03"
    );
}

#[test]
fn duration_record_constructor_rejects_invalid_fields_and_format_values() {
    assert_eq!(
        status_for("RETURN DURATION({foo: 1}) AS value"),
        GqlStatus::INVALID_DURATION_FUNCTION_FIELD_NAME
    );
    assert_eq!(
        status_for("RETURN DURATION({year: 1}) AS value"),
        GqlStatus::INVALID_DURATION_FUNCTION_FIELD_NAME
    );
    assert_eq!(
        status_for("RETURN DURATION({years: 1, days: 2}) AS value"),
        GqlStatus::INVALID_DURATION_FUNCTION_FIELD_NAME
    );
    assert_eq!(
        status_for("RETURN DURATION({seconds: 1, milliseconds: 2, nanoseconds: 3}) AS value"),
        GqlStatus::INVALID_DURATION_FUNCTION_FIELD_NAME
    );
    assert_eq!(
        status_for("RETURN DURATION({seconds: 'not-a-number'}) AS value"),
        GqlStatus::INVALID_DURATION_FORMAT
    );
}
