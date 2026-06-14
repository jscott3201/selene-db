//! Temporal keyword literal conformance cases.

use selene_core::{GraphId, Value};
use selene_gql::{EmptyProcedureRegistry, GqlStatus, ParserError, Session, StatementOutput, parse};
use selene_graph::SharedGraph;

fn single_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_811));
    let mut session = Session::new(&graph);
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("temporal literal query succeeds");
    let StatementOutput::Rows(table) = output else {
        panic!("expected row output");
    };
    assert_eq!(table.row_count(), 1, "expected one row");
    table.rows()[0].values()[0].clone()
}

fn assert_syntax_error(source: &str) {
    let error = parse(source).expect_err(source);
    assert!(
        matches!(error, ParserError::SyntaxError { .. }),
        "{source} must reject as syntax, got {error:?}"
    );
}

#[test]
fn date_and_local_temporal_literals_lower_to_values() {
    assert_eq!(
        single_value("RETURN DATE '2026-05-07' AS v"),
        Value::Date("2026-05-07".parse().unwrap())
    );
    assert_eq!(
        single_value("RETURN DATE @'2026-05-07' AS v"),
        Value::Date("2026-05-07".parse().unwrap())
    );
    assert_eq!(
        single_value("RETURN LOCAL DATETIME '2026-05-07T12:34:56' AS v"),
        Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap())
    );
    assert_eq!(
        single_value("RETURN DATETIME '2026-05-07T12:34:56' AS v"),
        Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap())
    );
    assert_eq!(
        single_value("RETURN LOCAL TIME '12:34:56' AS v"),
        Value::LocalTime("12:34:56".parse().unwrap())
    );
    assert_eq!(
        single_value("RETURN TIME '12:34:56' AS v"),
        Value::LocalTime("12:34:56".parse().unwrap())
    );
}

#[test]
fn zoned_temporal_literals_lower_to_values() {
    let Value::ZonedDateTime(value) =
        single_value("RETURN ZONED DATETIME '2026-05-07T12:34:56-04:00' AS v")
    else {
        panic!("expected zoned datetime");
    };
    assert_eq!(value.datetime().to_string(), "2026-05-07T12:34:56");
    assert_eq!(value.offset().to_string(), "-04");

    let Value::ZonedDateTime(value) =
        single_value("RETURN TIMESTAMP '2026-05-07T12:34:56-04:00' AS v")
    else {
        panic!("expected zoned datetime");
    };
    assert_eq!(value.datetime().to_string(), "2026-05-07T12:34:56");
    assert_eq!(value.offset().to_string(), "-04");

    let Value::ZonedTime(value) = single_value("RETURN TIME '12:34:56-04:00' AS v") else {
        panic!("expected zoned time");
    };
    assert_eq!(value.time().to_string(), "12:34:56");
    assert_eq!(value.offset().to_string(), "-04");
}

#[test]
fn temporal_literal_keywords_require_boundaries() {
    for source in [
        "RETURN ZONEDDATETIME '2026-05-07T12:34:56-04:00' AS v",
        "RETURN LOCALDATETIME '2026-05-07T12:34:56' AS v",
        "RETURN ZONEDTIME '12:34:56-04:00' AS v",
        "RETURN LOCALTIME '12:34:56' AS v",
        "RETURN DATETIMEx '2026-05-07T12:34:56' AS v",
        "RETURN TIMESTAMPx '2026-05-07T12:34:56-04:00' AS v",
        "RETURN DATEy '2026-05-07' AS v",
        "RETURN TIMEx '12:34:56' AS v",
        "RETURN DURATIONx 'PT1H' AS v",
        "RETURN DURATION_BETWEEN(DATE('2026-01-01'), DATE('2026-03-01')) YEARTOMONTH AS v",
        "RETURN DURATION_BETWEEN(DATE('2026-01-01'), DATE('2026-01-02')) DAYTOSECOND AS v",
    ] {
        assert_syntax_error(source);
    }
}

#[test]
fn guarded_temporal_keywords_still_accept_separated_forms() {
    for source in [
        "RETURN ZONED /* c */ DATETIME '2026-05-07T12:34:56-04:00' AS v",
        "RETURN LOCAL /* c */ DATETIME '2026-05-07T12:34:56' AS v",
        "RETURN ZONED /* c */ TIME '12:34:56-04:00' AS v",
        "RETURN LOCAL /* c */ TIME '12:34:56' AS v",
        "RETURN DURATION_BETWEEN(DATE('2026-01-01'), DATE('2026-03-01')) YEAR /* c */ TO /* c */ MONTH AS v",
        "RETURN DURATION_BETWEEN(DATE('2026-01-01'), DATE('2026-01-02')) DAY /* c */ TO /* c */ SECOND AS v",
    ] {
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    }
}

#[test]
fn duration_literal_forms_lower_to_values() {
    assert_eq!(
        single_value("RETURN DURATION 'PT1H2S' AS v"),
        Value::Duration(Box::new("PT1H2S".parse().unwrap()))
    );
    assert_eq!(
        single_value("RETURN DURATION('P2M') AS v"),
        Value::Duration(Box::new("P2M".parse().unwrap()))
    );
}

#[test]
fn invalid_temporal_literal_formats_report_data_exception() {
    for source in [
        "RETURN DATE 'not-date'",
        "RETURN LOCAL DATETIME '2026-05-07T12:34:56-04:00'",
        "RETURN ZONED TIME '12:34:56'",
    ] {
        let err = parse(source).expect_err("temporal format should be rejected");
        assert_eq!(err.gqlstatus(), GqlStatus::INVALID_DATETIME_FORMAT);
    }
}

#[test]
fn invalid_duration_literal_format_reports_duration_status() {
    let err = parse("RETURN DURATION 'not-duration'").expect_err("duration should be rejected");
    assert_eq!(err.gqlstatus(), GqlStatus::INVALID_DURATION_FORMAT);
}
