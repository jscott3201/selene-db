//! Temporal CAST conformance cases.

use selene_core::{GraphId, Value, intern};
use selene_gql::{EmptyProcedureRegistry, GqlStatus, Session, StatementOutput};
use selene_graph::SharedGraph;

fn cast_bound(value: Value, target: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_810));
    let mut session = Session::new(&graph);
    session.bind_parameter(intern("p").expect("intern param"), value);
    let source = format!("RETURN CAST($p AS {target}) AS v");
    let output = session
        .execute_source(&source, &EmptyProcedureRegistry)
        .expect("temporal cast succeeds");
    let StatementOutput::Rows(table) = output else {
        panic!("temporal cast produced non-row output");
    };
    table.rows()[0].values()[0].clone()
}

fn cast_bound_to_string(value: Value) -> String {
    let Value::String(value) = cast_bound(value, "STRING") else {
        panic!("temporal string cast did not return STRING");
    };
    value.as_str().to_owned()
}

fn cast_string(text: &str, target: &str) -> Value {
    cast_bound(Value::String(intern(text).expect("intern source")), target)
}

fn cast_string_status(text: &str, target: &str) -> GqlStatus {
    let graph = SharedGraph::new(GraphId::new(13_812));
    let mut session = Session::new(&graph);
    session.bind_parameter(
        intern("p").expect("intern param"),
        Value::String(intern(text).expect("intern source")),
    );
    let source = format!("RETURN CAST($p AS {target}) AS v");
    session
        .execute_source(&source, &EmptyProcedureRegistry)
        .expect_err("temporal cast should be rejected")
        .gqlstatus()
}

#[test]
fn cast_temporal_instants_to_strings() {
    assert_eq!(
        cast_bound_to_string(Value::Date("2026-05-07".parse().unwrap())),
        "2026-05-07"
    );
    assert_eq!(
        cast_bound_to_string(Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap())),
        "2026-05-07T12:34:56"
    );
    assert_eq!(
        cast_bound_to_string(Value::LocalTime("12:34:56".parse().unwrap())),
        "12:34:56"
    );
}

#[test]
fn cast_zoned_temporal_instants_omit_zone_annotation() {
    let zoned = "2026-05-07T12:34:56-04:00[America/New_York]";
    assert_eq!(
        cast_bound_to_string(Value::ZonedDateTime(Box::new(zoned.parse().unwrap()))),
        "2026-05-07T12:34:56-04"
    );
    assert_eq!(
        cast_bound_to_string(Value::ZonedTime(Box::new(zoned.parse().unwrap()))),
        "12:34:56-04"
    );
}

#[test]
fn cast_durations_to_iso_strings() {
    assert_eq!(
        cast_bound_to_string(Value::Duration(Box::new("P2M".parse().unwrap()))),
        "P2M"
    );
    assert_eq!(
        cast_bound_to_string(Value::Duration(Box::new("PT1H2S".parse().unwrap()))),
        "PT1H2S"
    );
}

#[test]
fn cast_strings_to_temporal_values() {
    assert_eq!(
        cast_string("2026-05-07", "DATE"),
        Value::Date("2026-05-07".parse().unwrap())
    );
    assert_eq!(
        cast_string(" 2026-05-07 ", "DATE"),
        Value::Date("2026-05-07".parse().unwrap())
    );
    assert_eq!(
        cast_string("2026-05-07T12:34:56", "LOCAL DATETIME"),
        Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap())
    );
    assert_eq!(
        cast_string("12:34:56", "LOCAL TIME"),
        Value::LocalTime("12:34:56".parse().unwrap())
    );
    assert_eq!(
        cast_string("PT1H2S", "DURATION"),
        Value::Duration(Box::new("PT1H2S".parse().unwrap()))
    );
    assert_eq!(
        cast_string(" PT1H2S ", "DURATION"),
        Value::Duration(Box::new("PT1H2S".parse().unwrap()))
    );

    let Value::ZonedDateTime(value) = cast_string("2026-05-07T12:34:56-04:00", "ZONED DATETIME")
    else {
        panic!("expected zoned datetime");
    };
    assert_eq!(value.datetime().to_string(), "2026-05-07T12:34:56");
    assert_eq!(value.offset().to_string(), "-04");

    let Value::ZonedTime(value) = cast_string("12:34:56-04:00", "ZONED TIME") else {
        panic!("expected zoned time");
    };
    assert_eq!(value.time().to_string(), "12:34:56");
    assert_eq!(value.offset().to_string(), "-04");
}

#[test]
fn cast_between_deterministic_temporal_instants() {
    assert_eq!(
        cast_bound(
            Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap()),
            "DATE"
        ),
        Value::Date("2026-05-07".parse().unwrap())
    );
    assert_eq!(
        cast_bound(
            Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap()),
            "LOCAL TIME"
        ),
        Value::LocalTime("12:34:56".parse().unwrap())
    );
    assert_eq!(
        cast_bound(Value::Date("2026-05-07".parse().unwrap()), "LOCAL DATETIME"),
        Value::LocalDateTime("2026-05-07T00:00:00".parse().unwrap())
    );

    let zoned_datetime = "2026-05-07T12:34:56-04:00[America/New_York]";
    assert_eq!(
        cast_bound(
            Value::ZonedDateTime(Box::new(zoned_datetime.parse().unwrap())),
            "DATE"
        ),
        Value::Date("2026-05-07".parse().unwrap())
    );
    assert_eq!(
        cast_bound(
            Value::ZonedDateTime(Box::new(zoned_datetime.parse().unwrap())),
            "LOCAL DATETIME"
        ),
        Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap())
    );

    let Value::ZonedTime(value) = cast_bound(
        Value::ZonedDateTime(Box::new(zoned_datetime.parse().unwrap())),
        "ZONED TIME",
    ) else {
        panic!("expected zoned time");
    };
    assert_eq!(value.time().to_string(), "12:34:56");
    assert_eq!(value.offset().to_string(), "-04");
}

#[test]
fn cast_invalid_strings_to_temporal_values_returns_22007() {
    assert_eq!(
        cast_string_status("not-date", "DATE"),
        GqlStatus::INVALID_DATETIME_FORMAT
    );
    assert_eq!(
        cast_string_status("2026-05-07T12:34:56-04:00", "LOCAL DATETIME"),
        GqlStatus::INVALID_DATETIME_FORMAT
    );
    assert_eq!(
        cast_string_status("2026-05-07T12:34:56", "ZONED DATETIME"),
        GqlStatus::INVALID_DATETIME_FORMAT
    );
    assert_eq!(
        cast_string_status("12:34:56", "ZONED TIME"),
        GqlStatus::INVALID_DATETIME_FORMAT
    );
}

#[test]
fn cast_invalid_strings_to_duration_returns_22g0h() {
    assert_eq!(
        cast_string_status("not-duration", "DURATION"),
        GqlStatus::INVALID_DURATION_FORMAT
    );
}
