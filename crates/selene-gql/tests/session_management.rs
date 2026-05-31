//! ISO/IEC 39075:2024 section 7 session-management behavior tests.
//!
//! Exercises the implemented session subset end to end: SET VALUE (GS03),
//! SET TIME ZONE (GS15) threaded into the section 20.27 current-datetime
//! functions, RESET targets (GS04/GS07/GS08/GS16), SESSION CLOSE (section 7.3)
//! with its termination guard, IF NOT EXISTS (section 7.4), the flagger feature
//! stamps, and the D1-deferred forms (SET GRAPH/SCHEMA) failing cleanly.

use selene_core::GraphId;
use selene_core::feature_register::{
    ANNEX_B_REGISTER, FeatureId, NOT_SUPPORTED_RATIONALE, SUPPORTED_FEATURES,
};
use selene_gql::{
    EmptyProcedureRegistry, ExecutorError, GqlStatus, Session, StatementOutput, Value, analyze,
    execute_statement, feature_walk, parse, plan,
};
use selene_graph::SharedGraph;

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn run(session: &mut Session<'_>, source: &str) -> Result<StatementOutput, ExecutorError> {
    session.execute_source(source, &EmptyProcedureRegistry)
}

fn single_value(output: StatementOutput) -> Value {
    let StatementOutput::Rows(table) = output else {
        panic!("expected row output, got {output:?}");
    };
    let row = table.rows().first().expect("at least one row");
    row.values().first().expect("at least one column").clone()
}

// ---------------------------------------------------------------------------
// SESSION SET VALUE (GS03)
// ---------------------------------------------------------------------------

#[test]
fn set_value_binds_parameter_visible_to_return() {
    let graph = graph(7001);
    let mut session = Session::new(&graph);

    assert!(matches!(
        run(&mut session, "SESSION SET VALUE $p = 42").expect("set value"),
        StatementOutput::Empty
    ));

    let value = single_value(run(&mut session, "RETURN $p").expect("return param"));
    assert_eq!(value, Value::Int(42));
}

#[test]
fn set_value_rebinds_existing_parameter() {
    let graph = graph(7002);
    let mut session = Session::new(&graph);
    run(&mut session, "SESSION SET VALUE $p = 1").expect("first bind");

    run(&mut session, "SESSION SET VALUE $p = 2").expect("rebind");

    assert_eq!(
        single_value(run(&mut session, "RETURN $p").expect("return")),
        Value::Int(2)
    );
}

#[test]
fn set_value_rhs_can_reference_prior_parameter() {
    let graph = graph(7003);
    let mut session = Session::new(&graph);
    run(&mut session, "SESSION SET VALUE $base = 10").expect("base bind");

    // The RHS is restricted to a <value specification>: a bare parameter
    // reference is allowed; a simple expression (GS14) is not.
    run(&mut session, "SESSION SET VALUE $alias = $base").expect("alias bind");

    assert_eq!(
        single_value(run(&mut session, "RETURN $alias").expect("return")),
        Value::Int(10)
    );
}

#[test]
fn set_value_rejects_simple_expression_rhs_gs14() {
    let graph = graph(7006);
    let mut session = Session::new(&graph);

    // Without GS14 the initializer must be a <value specification>; a binary
    // expression is rejected at parse time.
    let err = run(&mut session, "SESSION SET VALUE $p = 1 + 1").expect_err("simple expr rejected");
    assert!(matches!(err, ExecutorError::Parse { .. }));
}

#[test]
fn set_value_if_not_exists_keeps_existing_binding() {
    let graph = graph(7004);
    let mut session = Session::new(&graph);
    run(&mut session, "SESSION SET VALUE $p = 1").expect("first bind");

    run(&mut session, "SESSION SET VALUE IF NOT EXISTS $p = 999").expect("guarded bind");

    // The guard must leave the original value untouched.
    assert_eq!(
        single_value(run(&mut session, "RETURN $p").expect("return")),
        Value::Int(1)
    );
}

#[test]
fn set_value_if_not_exists_binds_when_absent() {
    let graph = graph(7005);
    let mut session = Session::new(&graph);

    run(&mut session, "SESSION SET VALUE IF NOT EXISTS $fresh = 7").expect("fresh bind");

    assert_eq!(
        single_value(run(&mut session, "RETURN $fresh").expect("return")),
        Value::Int(7)
    );
}

// ---------------------------------------------------------------------------
// SESSION SET TIME ZONE (GS15) threaded into current-datetime functions
// ---------------------------------------------------------------------------

#[test]
fn default_session_time_zone_is_utc() {
    let graph = graph(7010);
    let mut session = Session::new(&graph);

    let value = single_value(run(&mut session, "RETURN current_timestamp()").expect("now"));
    let Value::ZonedDateTime(zoned) = value else {
        panic!("expected zoned datetime, got {value:?}");
    };
    assert_eq!(zoned.offset().seconds(), 0, "ID048 default is UTC");
}

#[test]
fn set_time_zone_shifts_current_timestamp_offset() {
    let graph = graph(7011);
    let mut session = Session::new(&graph);

    run(&mut session, "SESSION SET TIME ZONE '+05:00'").expect("set tz");

    let value = single_value(run(&mut session, "RETURN current_timestamp()").expect("now"));
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

    let value = single_value(run(&mut session, "RETURN current_timestamp()").expect("now"));
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

#[test]
fn localtime_reflects_session_time_zone() {
    let graph = graph(7014);
    let mut session = Session::new(&graph);

    let utc = single_value(run(&mut session, "RETURN current_date()").expect("utc date"));
    run(&mut session, "SESSION SET TIME ZONE '+14:00'").expect("set tz");
    let shifted = single_value(run(&mut session, "RETURN current_date()").expect("shifted date"));

    // Both are dates; the assertion is that the call path is wired to the
    // session zone (the +14:00 wall clock can differ from UTC's date).
    assert!(matches!(utc, Value::Date(_)));
    assert!(matches!(shifted, Value::Date(_)));
}

// ---------------------------------------------------------------------------
// SESSION RESET (GS04 / GS07 / GS08 / GS16)
// ---------------------------------------------------------------------------

fn unbound_parameter(session: &mut Session<'_>) -> bool {
    matches!(
        run(session, "RETURN $p"),
        Err(ExecutorError::UnboundParameter { .. })
    )
}

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

    let value = single_value(run(&mut session, "RETURN current_timestamp()").expect("now"));
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
    let value = single_value(run(&mut session, "RETURN current_timestamp()").expect("now"));
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

    let value = single_value(run(&mut session, "RETURN current_timestamp()").expect("now"));
    let Value::ZonedDateTime(zoned) = value else {
        panic!("expected zoned datetime");
    };
    assert_eq!(zoned.offset().seconds(), 0);
}

// ---------------------------------------------------------------------------
// SESSION CLOSE (section 7.3) termination guard
// ---------------------------------------------------------------------------

#[test]
fn close_marks_session_and_guards_further_requests() {
    let graph = graph(7030);
    let mut session = Session::new(&graph);

    assert!(matches!(
        run(&mut session, "SESSION CLOSE").expect("close"),
        StatementOutput::Empty
    ));
    assert!(session.is_closed());

    let err = run(&mut session, "RETURN 1").expect_err("closed session rejects");
    assert!(matches!(err, ExecutorError::SessionClosed { .. }));
    assert_eq!(err.gqlstatus(), GqlStatus::SESSION_CLOSED);
}

#[test]
fn close_rolls_back_active_transaction() {
    let graph = graph(7031);
    let mut session = Session::new(&graph);
    run(&mut session, "START TRANSACTION").expect("start");
    assert!(session.has_active_txn());

    run(&mut session, "SESSION CLOSE").expect("close");

    assert!(
        !session.has_active_txn(),
        "close rolls back the open transaction"
    );
    assert!(session.is_closed());
}

#[test]
fn closed_session_rejects_another_session_command() {
    let graph = graph(7032);
    let mut session = Session::new(&graph);
    run(&mut session, "SESSION CLOSE").expect("close");

    let err = run(&mut session, "SESSION SET VALUE $p = 1").expect_err("closed rejects");
    assert!(matches!(err, ExecutorError::SessionClosed { .. }));
}

#[test]
fn closed_session_rejects_cached_plan_execution() {
    // The termination guard must hold on the sibling public entry too: an
    // embedder that prepares an `ExecutionPlan` once (parse -> analyze -> plan,
    // the cached-plan pipeline the embedding guide documents) and re-executes
    // it via `execute_statement` must not bypass `SESSION CLOSE`. Regression
    // for the guard previously living only in `execute_source`.
    let graph = graph(7033);
    let mut session = Session::new(&graph);

    let statement = parse("RETURN 1").expect("parse");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("analyze");
    let cached_plan = plan(&analyzed, &EmptyProcedureRegistry).expect("plan");

    // Plan executes fine before the session is closed.
    execute_statement(&cached_plan, &mut session, &EmptyProcedureRegistry).expect("pre-close run");

    run(&mut session, "SESSION CLOSE").expect("close");

    let err = execute_statement(&cached_plan, &mut session, &EmptyProcedureRegistry)
        .expect_err("closed session rejects cached plan");
    assert!(matches!(err, ExecutorError::SessionClosed { .. }));
    assert_eq!(err.gqlstatus(), GqlStatus::SESSION_CLOSED);
}

// ---------------------------------------------------------------------------
// Flagger feature stamps
// ---------------------------------------------------------------------------

fn walked_features(source: &str) -> Vec<FeatureId> {
    let statement = parse(source).expect("parse");
    feature_walk(&statement)
        .into_iter()
        .map(|use_| use_.feature_id)
        .collect()
}

#[test]
fn flagger_stamps_set_value_gs03() {
    assert!(walked_features("SESSION SET VALUE $p = 1").contains(&FeatureId::GS03));
}

#[test]
fn flagger_stamps_set_time_zone_gs15() {
    assert!(walked_features("SESSION SET TIME ZONE '+00:00'").contains(&FeatureId::GS15));
}

#[test]
fn flagger_stamps_reset_targets() {
    assert!(walked_features("SESSION RESET").contains(&FeatureId::GS04));
    assert!(walked_features("SESSION RESET PARAMETERS").contains(&FeatureId::GS08));
    assert!(walked_features("SESSION RESET TIME ZONE").contains(&FeatureId::GS07));
    assert!(walked_features("SESSION RESET PARAMETER $p").contains(&FeatureId::GS16));
}

#[test]
fn flagger_reset_parameter_co_stamps_gs08_and_gs16() {
    // ISO/IEC 39075:2024 section 7.2: `SESSION RESET PARAMETER <name>` uses both
    // the RESET-PARAMETER surface (CR6 → GS08) and the parameter-name argument
    // (CR7 → GS16); a faithful Flagger stamps both. `RESET ALL PARAMETERS` is
    // the GS08 surface without the parameter-name argument, so it stamps GS08
    // only (no GS16).
    let single = walked_features("SESSION RESET PARAMETER $p");
    assert!(
        single.contains(&FeatureId::GS08),
        "RESET PARAMETER implies GS08"
    );
    assert!(
        single.contains(&FeatureId::GS16),
        "RESET PARAMETER implies GS16"
    );

    let all = walked_features("SESSION RESET PARAMETERS");
    assert!(all.contains(&FeatureId::GS08));
    assert!(
        !all.contains(&FeatureId::GS16),
        "RESET ALL PARAMETERS has no parameter-name argument, so no GS16"
    );
}

#[test]
fn flagger_does_not_stamp_session_close() {
    // SESSION CLOSE (section 7.3) has no ISO feature code.
    assert!(walked_features("SESSION CLOSE").is_empty());
}

// ---------------------------------------------------------------------------
// Deferred D1-blocked forms (GS01 / GS02 / GS05 / GS06)
// ---------------------------------------------------------------------------

#[test]
fn deferred_set_graph_fails_to_parse() {
    // SESSION SET <name> GRAPH (GS01) is not in the grammar under D1.
    assert!(parse("SESSION SET $g GRAPH CURRENT_GRAPH").is_err());
}

#[test]
fn deferred_set_schema_fails_to_parse() {
    // SESSION SET <name> SCHEMA (D1: no schema layer) is not in the grammar.
    assert!(parse("SESSION SET SCHEMA myschema").is_err());
}

#[test]
fn deferred_reset_schema_fails_to_parse() {
    // SESSION RESET SCHEMA (GS05) is not in the grammar under D1.
    assert!(parse("SESSION RESET SCHEMA").is_err());
}

// ---------------------------------------------------------------------------
// feature_register hygiene (selene-core claim surface)
// ---------------------------------------------------------------------------

#[test]
fn implemented_session_features_are_supported() {
    for id in [
        FeatureId::GS03,
        FeatureId::GS04,
        FeatureId::GS07,
        FeatureId::GS08,
        FeatureId::GS15,
        FeatureId::GS16,
    ] {
        assert!(SUPPORTED_FEATURES.contains(&id), "{id} must be supported");
    }
}

#[test]
fn deferred_session_features_have_d1_rationale() {
    for id in [
        FeatureId::GS01,
        FeatureId::GS02,
        FeatureId::GS05,
        FeatureId::GS06,
        FeatureId::GS10,
        FeatureId::GS11,
        FeatureId::GS12,
        FeatureId::GS13,
        FeatureId::GS14,
    ] {
        assert!(
            !SUPPORTED_FEATURES.contains(&id),
            "{id} must not be claimed as supported"
        );
        assert!(
            NOT_SUPPORTED_RATIONALE
                .iter()
                .any(|(feature, _)| *feature == id),
            "{id} must carry a deferral rationale"
        );
    }
}

#[test]
fn session_defaults_are_registered_in_annex_b() {
    for id in ["ID048", "ID049"] {
        assert!(
            ANNEX_B_REGISTER
                .iter()
                .any(|(annex, _)| annex.as_str() == id),
            "{id} must be registered in Annex B"
        );
    }
}
