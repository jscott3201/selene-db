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

fn cast_bound_in_zone(value: Value, target: &str, zone: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_811));
    let mut session = Session::new(&graph);
    session
        .execute_source(
            &format!("SESSION SET TIME ZONE '{zone}'"),
            &EmptyProcedureRegistry,
        )
        .expect("set session time zone");
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

fn current_date(session: &mut Session<'_>) -> jiff::civil::Date {
    let output = session
        .execute_source("RETURN current_date() AS d", &EmptyProcedureRegistry)
        .expect("current date");
    let StatementOutput::Rows(table) = output else {
        panic!("current_date produced non-row output");
    };
    let Value::Date(value) = table.rows()[0].values()[0].clone() else {
        panic!("current_date produced non-date value");
    };
    value
}

fn cast_bound_with_date_window(
    session: &mut Session<'_>,
    value: Value,
    target: &str,
) -> (Value, jiff::civil::Date, jiff::civil::Date) {
    let before = current_date(session);
    session.bind_parameter(intern("p").expect("intern param"), value);
    let source = format!("RETURN CAST($p AS {target}) AS v");
    let output = session
        .execute_source(&source, &EmptyProcedureRegistry)
        .expect("temporal cast succeeds");
    let after = current_date(session);
    let StatementOutput::Rows(table) = output else {
        panic!("temporal cast produced non-row output");
    };
    (table.rows()[0].values()[0].clone(), before, after)
}

fn assert_current_date_window(
    actual: jiff::civil::Date,
    before: jiff::civil::Date,
    after: jiff::civil::Date,
) {
    assert!(
        actual == before || actual == after,
        "expected {actual} to match current-date window {before}..={after}"
    );
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
        Value::LocalDateTime("2026-05-07T08:34:56".parse().unwrap())
    );
    assert_eq!(
        cast_bound(
            Value::ZonedDateTime(Box::new(zoned_datetime.parse().unwrap())),
            "LOCAL TIME"
        ),
        Value::LocalTime("08:34:56".parse().unwrap())
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
fn cast_local_temporals_to_zoned_use_session_time_zone() {
    let Value::ZonedDateTime(value) = cast_bound_in_zone(
        Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap()),
        "ZONED DATETIME",
        "+05:30",
    ) else {
        panic!("expected zoned datetime");
    };
    assert_eq!(value.datetime().to_string(), "2026-05-07T12:34:56");
    assert_eq!(value.offset().seconds(), 5 * 3600 + 30 * 60);

    let Value::ZonedDateTime(value) = cast_bound_in_zone(
        Value::Date("2026-05-07".parse().unwrap()),
        "ZONED DATETIME",
        "-04:00",
    ) else {
        panic!("expected zoned datetime");
    };
    assert_eq!(value.datetime().to_string(), "2026-05-07T00:00:00");
    assert_eq!(value.offset().seconds(), -4 * 3600);

    let Value::ZonedTime(value) = cast_bound_in_zone(
        Value::LocalTime("12:34:56".parse().unwrap()),
        "ZONED TIME",
        "+02:30",
    ) else {
        panic!("expected zoned time");
    };
    assert_eq!(value.time().to_string(), "12:34:56");
    assert_eq!(value.offset().seconds(), 2 * 3600 + 30 * 60);
}

#[test]
fn cast_list_elements_preserve_session_time_zone() {
    let value = Value::List(vec![Value::LocalDateTime(
        "2026-05-07T12:34:56".parse().unwrap(),
    )]);
    let Value::List(values) = cast_bound_in_zone(value, "LIST<ZONED DATETIME>", "+01:15") else {
        panic!("expected list");
    };
    let [Value::ZonedDateTime(value)] = values.as_slice() else {
        panic!("expected one zoned datetime");
    };
    assert_eq!(value.datetime().to_string(), "2026-05-07T12:34:56");
    assert_eq!(value.offset().seconds(), 3600 + 15 * 60);
}

#[test]
fn cast_times_to_datetimes_use_current_session_date() {
    let graph = SharedGraph::new(GraphId::new(13_813));
    let mut session = Session::new(&graph);
    session
        .execute_source("SESSION SET TIME ZONE '+09:00'", &EmptyProcedureRegistry)
        .expect("set session time zone");

    let local_time = "23:45:01".parse().unwrap();
    let (value, before, after) =
        cast_bound_with_date_window(&mut session, Value::LocalTime(local_time), "LOCAL DATETIME");
    let Value::LocalDateTime(value) = value else {
        panic!("expected local datetime");
    };
    assert_eq!(value.time(), local_time);
    assert_current_date_window(value.date(), before, after);

    let local_time = "08:09:10".parse().unwrap();
    let (value, before, after) =
        cast_bound_with_date_window(&mut session, Value::LocalTime(local_time), "ZONED DATETIME");
    let Value::ZonedDateTime(value) = value else {
        panic!("expected zoned datetime");
    };
    assert_eq!(value.time(), local_time);
    assert_eq!(value.offset().seconds(), 9 * 3600);
    assert_current_date_window(value.date(), before, after);

    let zoned_time = Value::ZonedTime(Box::new(
        "1970-01-01T08:09:10-07:00[Etc/GMT+7]".parse().unwrap(),
    ));
    let Value::LocalTime(value) = cast_bound(zoned_time.clone(), "LOCAL TIME") else {
        panic!("expected local time");
    };
    assert_eq!(value.to_string(), "01:09:10");

    let (value, before, after) =
        cast_bound_with_date_window(&mut session, zoned_time.clone(), "LOCAL DATETIME");
    let Value::LocalDateTime(value) = value else {
        panic!("expected local datetime");
    };
    assert_eq!(value.time().to_string(), "01:09:10");
    assert_current_date_window(value.date(), before, after);

    let (value, before, after) =
        cast_bound_with_date_window(&mut session, zoned_time, "ZONED DATETIME");
    let Value::ZonedDateTime(value) = value else {
        panic!("expected zoned datetime");
    };
    assert_eq!(value.time().to_string(), "08:09:10");
    assert_eq!(value.offset().seconds(), -7 * 3600);
    assert_current_date_window(value.date(), before, after);
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
