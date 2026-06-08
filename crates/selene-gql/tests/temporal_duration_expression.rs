//! Temporal instant plus duration value-expression coverage.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, execute_read, execute_read_result};
use selene_core::Value;
use selene_gql::GqlStatus;

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

#[test]
fn temporal_duration_addition_and_subtraction_preserve_temporal_family() {
    assert_eq!(
        single_value(
            "RETURN DATE('2026-01-01') + DURATION('P2D') AS value",
            "value"
        ),
        Value::Date("2026-01-03".parse().unwrap())
    );
    assert_eq!(
        single_value(
            "RETURN DURATION('P2D') + DATE('2026-01-01') AS value",
            "value"
        ),
        Value::Date("2026-01-03".parse().unwrap())
    );
    assert_eq!(
        single_value(
            "RETURN DATE('2026-01-03') - DURATION('P2D') AS value",
            "value"
        ),
        Value::Date("2026-01-01".parse().unwrap())
    );
    assert_eq!(
        single_value(
            "RETURN LOCAL_DATETIME('2026-01-01T01:00:00') + DURATION('PT2H30M') AS value",
            "value"
        ),
        Value::LocalDateTime("2026-01-01T03:30:00".parse().unwrap())
    );
    assert_eq!(
        single_value(
            "RETURN LOCAL_TIME('12:00:00') + DURATION('PT2H30M') AS value",
            "value"
        ),
        Value::LocalTime("14:30:00".parse().unwrap())
    );
}

#[test]
fn zoned_temporal_duration_arithmetic_preserves_offset() {
    let value = single_value(
        "RETURN ZONED_DATETIME('2026-01-01T01:00:00-04:00') + DURATION('PT2H') AS value",
        "value",
    );
    let Value::ZonedDateTime(value) = value else {
        panic!("expected zoned datetime");
    };
    assert_eq!(value.datetime().to_string(), "2026-01-01T03:00:00");
    assert_eq!(value.offset().to_string(), "-04");

    let value = single_value(
        "RETURN ZONED_TIME('12:00:00-04:00') + DURATION('PT2H30M') AS value",
        "value",
    );
    let Value::ZonedTime(value) = value else {
        panic!("expected zoned time");
    };
    assert_eq!(value.time().to_string(), "14:30:00");
    assert_eq!(value.offset().to_string(), "-04");
}

#[test]
fn temporal_duration_arithmetic_preserves_null_semantics() {
    assert_eq!(
        single_value("RETURN DATE('2026-01-01') + NULL AS value", "value"),
        Value::Null
    );
    assert_eq!(
        single_value("RETURN NULL + DATE('2026-01-01') AS value", "value"),
        Value::Null
    );
    assert_eq!(
        single_value("RETURN LOCAL_TIME('12:00:00') - NULL AS value", "value"),
        Value::Null
    );
}

#[test]
fn temporal_duration_arithmetic_rejects_invalid_operand_shapes() {
    assert_eq!(
        status_for("RETURN DATE('2026-01-01') + 1 AS value"),
        GqlStatus::DATATYPE_MISMATCH
    );
    assert_eq!(
        status_for("RETURN DURATION('P1D') - DATE('2026-01-01') AS value"),
        GqlStatus::DATATYPE_MISMATCH
    );
    assert_eq!(
        status_for("RETURN LOCAL_TIME('23:59:59') + DURATION('PT1S') AS value"),
        GqlStatus::NUMERIC_VALUE_OUT_OF_RANGE
    );
}

#[test]
fn temporal_duration_literal_query_executes() {
    let table = execute_read_result(
        "RETURN DATE '2026-01-01' + DURATION 'P1D' AS date_value, \
         LOCAL DATETIME '2026-01-01T00:00:00' - DURATION 'PT1H' AS datetime_value",
    )
    .expect("literal temporal duration query succeeds");
    assert_eq!(
        table.rows()[0].values()[0],
        Value::Date("2026-01-02".parse().unwrap())
    );
    assert_eq!(
        table.rows()[0].values()[1],
        Value::LocalDateTime("2025-12-31T23:00:00".parse().unwrap())
    );
}
