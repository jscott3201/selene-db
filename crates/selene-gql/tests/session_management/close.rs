use super::*;

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
