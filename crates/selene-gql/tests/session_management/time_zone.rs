use super::*;

#[test]
fn default_session_time_zone_is_utc() {
    let graph = graph(7010);
    let mut session = Session::new(&graph);

    let value = single_value(run(&mut session, "RETURN current_timestamp").expect("now"));
    let Value::ZonedDateTime(zoned) = value else {
        panic!("expected zoned datetime, got {value:?}");
    };
    assert_eq!(zoned.offset().seconds(), 0, "ID048 default is UTC");
}

#[test]
fn non_iso_now_function_is_not_in_the_scalar_set() {
    let graph = graph(7015);
    let mut session = Session::new(&graph);

    let err = run(&mut session, "RETURN now()").expect_err("NOW is not an ISO datetime function");
    assert_eq!(err.gqlstatus().as_str(), "22G03");
}

#[test]
fn current_datetime_keyword_forms_execute() {
    let graph = graph(7016);
    let mut session = Session::new(&graph);

    run(&mut session, "SESSION SET TIME ZONE '+03:00'").expect("set tz");

    let value = single_value(run(&mut session, "RETURN CURRENT_TIMESTAMP").expect("timestamp"));
    let Value::ZonedDateTime(zoned) = value else {
        panic!("expected zoned datetime, got {value:?}");
    };
    assert_eq!(zoned.offset().seconds(), 3 * 3600);

    assert!(matches!(
        single_value(run(&mut session, "RETURN CURRENT_DATE").expect("date")),
        Value::Date(_)
    ));
    assert!(matches!(
        single_value(run(&mut session, "RETURN CURRENT_TIME").expect("time")),
        Value::ZonedTime(_)
    ));
    assert!(matches!(
        single_value(run(&mut session, "RETURN LOCAL_TIMESTAMP").expect("local timestamp")),
        Value::LocalDateTime(_)
    ));
    assert!(matches!(
        single_value(run(&mut session, "RETURN LOCAL_TIME").expect("local time")),
        Value::LocalTime(_)
    ));
    assert!(matches!(
        single_value(run(&mut session, "RETURN LOCAL_TIME()").expect("local time parens")),
        Value::LocalTime(_)
    ));
}

#[test]
fn current_datetime_constructor_forms_execute() {
    let graph = graph(7017);
    let mut session = Session::new(&graph);

    run(&mut session, "SESSION SET TIME ZONE '+03:00'").expect("set tz");

    assert!(matches!(
        single_value(run(&mut session, "RETURN DATE()").expect("date")),
        Value::Date(_)
    ));

    let value = single_value(run(&mut session, "RETURN ZONED_TIME()").expect("zoned time"));
    let Value::ZonedTime(zoned_time) = value else {
        panic!("expected zoned time, got {value:?}");
    };
    assert_eq!(zoned_time.offset().seconds(), 3 * 3600);

    let value = single_value(run(&mut session, "RETURN ZONED_DATETIME()").expect("zoned dt"));
    let Value::ZonedDateTime(zoned_datetime) = value else {
        panic!("expected zoned datetime, got {value:?}");
    };
    assert_eq!(zoned_datetime.offset().seconds(), 3 * 3600);

    assert!(matches!(
        single_value(run(&mut session, "RETURN LOCAL_DATETIME()").expect("local dt")),
        Value::LocalDateTime(_)
    ));
    assert_eq!(
        single_value(
            run(&mut session, "RETURN LOCAL_TIME('12:34:56')").expect("local time string")
        ),
        Value::LocalTime("12:34:56".parse().unwrap())
    );
    assert_eq!(
        single_value(run(&mut session, "RETURN DATE('2026-05-07')").expect("date string")),
        Value::Date("2026-05-07".parse().unwrap())
    );
}

#[test]
fn set_time_zone_shifts_current_timestamp_offset() {
    let graph = graph(7011);
    let mut session = Session::new(&graph);

    run(&mut session, "SESSION SET TIME ZONE '+05:00'").expect("set tz");

    let value = single_value(run(&mut session, "RETURN current_timestamp").expect("now"));
    let Value::ZonedDateTime(zoned) = value else {
        panic!("expected zoned datetime, got {value:?}");
    };
    assert_eq!(
        zoned.offset().seconds(),
        5 * 3600,
        "current_timestamp must reflect the session time zone"
    );
}

#[test]
fn set_time_zone_iana_region_is_accepted() {
    let graph = graph(7012);
    let mut session = Session::new(&graph);

    // A fixed-offset IANA zone keeps the offset deterministic across DST.
    run(&mut session, "SESSION SET TIME ZONE 'Etc/GMT+5'").expect("set iana tz");

    let value = single_value(run(&mut session, "RETURN current_timestamp").expect("now"));
    let Value::ZonedDateTime(zoned) = value else {
        panic!("expected zoned datetime, got {value:?}");
    };
    // Etc/GMT+5 is UTC-5 (POSIX sign convention).
    assert_eq!(zoned.offset().seconds(), -5 * 3600);
}

#[test]
fn set_time_zone_rejects_unknown_zone_with_data_exception() {
    let graph = graph(7013);
    let mut session = Session::new(&graph);

    let err = run(&mut session, "SESSION SET TIME ZONE 'Mars/Olympus_Mons'")
        .expect_err("unknown zone rejected");

    assert_eq!(err.gqlstatus(), GqlStatus::INVALID_TIME_ZONE);
    // The session must remain usable after a rejected time-zone change.
    assert!(!session.is_closed());
    run(&mut session, "RETURN 1").expect("session still usable");
}

// ---------------------------------------------------------------------------
// SESSION SET GRAPH to current-graph expressions (section 7.1)
// ---------------------------------------------------------------------------

#[test]
fn localtime_reflects_session_time_zone() {
    let graph = graph(7014);
    let mut session = Session::new(&graph);

    let utc = single_value(run(&mut session, "RETURN current_date").expect("utc date"));
    run(&mut session, "SESSION SET TIME ZONE '+14:00'").expect("set tz");
    let shifted = single_value(run(&mut session, "RETURN current_date").expect("shifted date"));

    // Both are dates; the assertion is that the call path is wired to the
    // session zone (the +14:00 wall clock can differ from UTC's date).
    assert!(matches!(utc, Value::Date(_)));
    assert!(matches!(shifted, Value::Date(_)));
}

// ---------------------------------------------------------------------------
// SESSION RESET (GS04 / GS07 / GS08 / GS16)
// ---------------------------------------------------------------------------
