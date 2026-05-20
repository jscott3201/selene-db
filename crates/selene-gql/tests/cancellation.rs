//! Cancellation, deadline, and per-statement row-cap integration tests.

use std::time::{Duration, Instant};

use selene_core::{CancellationToken, GraphId};
use selene_gql::{EmptyProcedureRegistry, ExecutorError, Session, StatementOutput};
use selene_graph::SharedGraph;

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn rows(output: StatementOutput) -> usize {
    match output {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("expected row output, got {other:?}"),
    }
}

#[test]
fn cancelled_token_short_circuits_statement() {
    let graph = graph(4117);
    let token = CancellationToken::new();
    token.cancel();
    let mut session = Session::new(&graph).with_cancellation_token(token);

    let err = session
        .execute_source("RETURN 1 AS n", &EmptyProcedureRegistry)
        .expect_err("cancelled token aborts before execution");

    assert!(matches!(err, ExecutorError::Cancelled { .. }));
    assert_eq!(err.gqlstatus().as_str(), "5GQL2");
}

#[test]
fn expired_deadline_returns_timeout() {
    let graph = graph(4118);
    let deadline = Instant::now() - Duration::from_millis(1);
    let mut session = Session::new(&graph).with_deadline(deadline);

    let err = session
        .execute_source("RETURN 1 AS n", &EmptyProcedureRegistry)
        .expect_err("expired deadline aborts before execution");

    let ExecutorError::Timeout {
        deadline: observed,
        elapsed,
        ..
    } = err
    else {
        panic!("expected timeout, got {err:?}");
    };
    assert_eq!(observed, deadline);
    assert!(elapsed >= Duration::ZERO);
}

#[test]
fn row_cap_counts_outermost_result_rows_only() {
    let graph = graph(4119);
    let source = "UNWIND [5, 1, 4, 2, 3] AS x RETURN x ORDER BY x LIMIT 2";
    let mut capped_at_result = Session::new(&graph).with_row_cap(2);

    let output = capped_at_result
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("outer result has exactly two rows");
    assert_eq!(rows(output), 2);

    let mut too_low = Session::new(&graph).with_row_cap(1);
    let err = too_low
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("outer result exceeds row cap");

    assert!(matches!(err, ExecutorError::RowCapExceeded { cap: 1, .. }));
    assert_eq!(err.gqlstatus().as_str(), "5GQL1");
}

#[test]
fn row_cap_ignores_non_row_bearing_writes() {
    let graph = graph(4120);
    let mut session = Session::new(&graph).with_row_cap(0);

    let output = session
        .execute_source("INSERT (:Person)", &EmptyProcedureRegistry)
        .expect("write without RETURN does not count internal rows");

    assert!(matches!(
        output,
        StatementOutput::Written(outcome) if outcome.rows.is_none()
    ));
}

#[test]
fn cancellation_aborts_explicit_transaction_until_rollback() {
    let graph = graph(4121);
    let token = CancellationToken::new();
    let mut session = Session::new(&graph).with_cancellation_token(token.clone());
    session.start_transaction().expect("start succeeds");
    token.cancel();

    let err = session
        .execute_source("RETURN 1 AS n", &EmptyProcedureRegistry)
        .expect_err("cancelled statement aborts explicit transaction");
    assert!(matches!(err, ExecutorError::Cancelled { .. }));
    assert!(session.is_aborted());

    let err = session
        .execute_source("RETURN 2 AS n", &EmptyProcedureRegistry)
        .expect_err("aborted transaction rejects non-control statement");
    assert!(matches!(err, ExecutorError::InFailedTransaction { .. }));

    session
        .execute_source("ROLLBACK", &EmptyProcedureRegistry)
        .expect("rollback recovers aborted transaction");
    assert!(!session.is_aborted());
    assert!(!session.has_active_txn());
}

#[test]
fn cancelled_session_does_not_poison_concurrent_session() {
    let graph = graph(4122);
    let token = CancellationToken::new();
    token.cancel();
    let mut cancelled = Session::new(&graph).with_cancellation_token(token);
    let mut normal = Session::new(&graph);

    let err = cancelled
        .execute_source("RETURN 1 AS n", &EmptyProcedureRegistry)
        .expect_err("cancelled session errors");
    assert!(matches!(err, ExecutorError::Cancelled { .. }));

    let output = normal
        .execute_source("RETURN 2 AS n", &EmptyProcedureRegistry)
        .expect("independent session still runs");
    assert_eq!(rows(output), 1);
}

#[test]
fn read_guard_releases_after_row_cap_error() {
    let graph = graph(4123);
    let mut capped = Session::new(&graph).with_row_cap(0);

    let err = capped
        .execute_source("RETURN 1 AS n", &EmptyProcedureRegistry)
        .expect_err("row cap errors at result boundary");
    assert!(matches!(err, ExecutorError::RowCapExceeded { .. }));

    let mut writer = Session::new(&graph);
    writer
        .execute_source("INSERT (:Person)", &EmptyProcedureRegistry)
        .expect("write succeeds after early read error releases guard");
}
