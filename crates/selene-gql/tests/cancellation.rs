//! Cancellation, deadline, and per-statement row-cap integration tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
fn mid_execution_cancellation_aborts_large_query_with_5gql2() {
    // GQLRT-29: existing tests all cancel BEFORE execute_source (the
    // pre-execution guard). This drives a >2048-row query (well past the
    // 1024-row cancellation stride) and cancels it from a background thread
    // while execution is in flight, exercising the in-loop stride checkpoint.
    // The query is wired through GROUP BY, whose accumulation loop checks the
    // cancellation stride per row.
    let row_count = 6000;
    let list = (0..row_count)
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let source = format!("FOR x IN [{list}] RETURN count(*) AS c");
    let graph = graph(4130);

    // Sanity: uncancelled the query completes with the full count.
    {
        let mut session = Session::new(&graph);
        let output = session
            .execute_source(&source, &EmptyProcedureRegistry)
            .expect("uncancelled large query runs to completion");
        match output {
            StatementOutput::Rows(table) => assert_eq!(table.row_count(), 1),
            other => panic!("expected rows, got {other:?}"),
        }
    }

    // Cancel from a background thread; loop a bounded number of attempts until
    // a cancellation lands (deterministic in practice — the worker cancels the
    // instant the main thread signals it is about to execute).
    let mut observed_cancel = false;
    for _ in 0..256 {
        let token = CancellationToken::new();
        let worker_token = token.clone();
        let about_to_run = Arc::new(AtomicBool::new(false));
        let worker_signal = about_to_run.clone();
        let canceller = std::thread::spawn(move || {
            while !worker_signal.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }
            worker_token.cancel();
        });

        let mut session = Session::new(&graph).with_cancellation_token(token);
        about_to_run.store(true, Ordering::Release);
        let result = session.execute_source(&source, &EmptyProcedureRegistry);
        canceller.join().expect("canceller thread joins");

        match result {
            Ok(_) => continue,
            Err(err) => {
                assert!(
                    matches!(err, ExecutorError::Cancelled { .. }),
                    "cancellation must surface as Cancelled, got {err:?}"
                );
                assert_eq!(err.gqlstatus().as_str(), "5GQL2");
                observed_cancel = true;
                break;
            }
        }
    }
    assert!(
        observed_cancel,
        "background cancellation of a {row_count}-row query never landed in 256 attempts"
    );
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
    let source = "FOR x IN [5, 1, 4, 2, 3] RETURN x ORDER BY x LIMIT 2";
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
