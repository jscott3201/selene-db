use super::*;

#[test]
fn reset_parameter_clears_single_binding() {
    let graph = graph(7020);
    let mut session = Session::new(&graph);
    run(&mut session, "SESSION SET VALUE $p = 1").expect("bind");
    run(&mut session, "SESSION SET VALUE $q = 2").expect("bind q");

    run(&mut session, "SESSION RESET PARAMETER $p").expect("reset p");

    assert!(unbound_parameter(&mut session), "$p must be cleared");
    assert_eq!(
        single_value(run(&mut session, "RETURN $q").expect("q survives")),
        Value::Int(2)
    );
}

#[test]
fn reset_parameter_without_keyword_clears_single_binding() {
    let graph = graph(7021);
    let mut session = Session::new(&graph);
    run(&mut session, "SESSION SET VALUE $p = 1").expect("bind");

    run(&mut session, "SESSION RESET $p").expect("reset p (no PARAMETER keyword)");

    assert!(unbound_parameter(&mut session));
}

#[test]
fn reset_parameters_clears_all_bindings() {
    let graph = graph(7022);
    let mut session = Session::new(&graph);
    run(&mut session, "SESSION SET VALUE $p = 1").expect("bind p");
    run(&mut session, "SESSION SET VALUE $q = 2").expect("bind q");

    run(&mut session, "SESSION RESET PARAMETERS").expect("reset all params");

    assert!(unbound_parameter(&mut session));
    assert!(matches!(
        run(&mut session, "RETURN $q"),
        Err(ExecutorError::UnboundParameter { .. })
    ));
}

#[test]
fn reset_time_zone_restores_utc_default() {
    let graph = graph(7023);
    let mut session = Session::new(&graph);
    run(&mut session, "SESSION SET TIME ZONE '+05:00'").expect("set tz");

    run(&mut session, "SESSION RESET TIME ZONE").expect("reset tz");

    let value = single_value(run(&mut session, "RETURN current_timestamp").expect("now"));
    let Value::ZonedDateTime(zoned) = value else {
        panic!("expected zoned datetime");
    };
    assert_eq!(zoned.offset().seconds(), 0, "reset returns to UTC");
}

#[test]
fn reset_all_characteristics_clears_params_and_time_zone() {
    let graph = graph(7024);
    let mut session = Session::new(&graph);
    run(&mut session, "SESSION SET VALUE $p = 1").expect("bind");
    run(&mut session, "SESSION SET TIME ZONE '+05:00'").expect("set tz");

    run(&mut session, "SESSION RESET").expect("bare reset = all characteristics");

    assert!(unbound_parameter(&mut session), "params cleared");
    let value = single_value(run(&mut session, "RETURN current_timestamp").expect("now"));
    let Value::ZonedDateTime(zoned) = value else {
        panic!("expected zoned datetime");
    };
    assert_eq!(zoned.offset().seconds(), 0, "time zone reset to UTC");
}

#[test]
fn reset_all_characteristics_explicit_keyword() {
    let graph = graph(7025);
    let mut session = Session::new(&graph);
    run(&mut session, "SESSION SET TIME ZONE '+05:00'").expect("set tz");

    run(&mut session, "SESSION RESET ALL CHARACTERISTICS").expect("explicit reset");

    let value = single_value(run(&mut session, "RETURN current_timestamp").expect("now"));
    let Value::ZonedDateTime(zoned) = value else {
        panic!("expected zoned datetime");
    };
    assert_eq!(zoned.offset().seconds(), 0);
}

// ---------------------------------------------------------------------------
// SESSION CLOSE (section 7.3) termination guard
// ---------------------------------------------------------------------------
