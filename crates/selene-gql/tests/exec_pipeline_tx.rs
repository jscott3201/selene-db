//! Defensive transaction-control pipeline tests.

use std::sync::Arc;

use selene_core::GraphId;
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, ExecutorError,
    ImplDefinedCaps, PipelineOp, Session, StatementOutput, TxContext, analyze, execute_pipeline,
    execute_statement, parse, plan,
};
use selene_graph::SharedGraph;

fn tx_op(source: &str) -> PipelineOp {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    let mut plan = plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans");
    plan.pipeline.remove(0)
}

fn execute(source: &str, session: &mut Session<'_>) -> Result<StatementOutput, ExecutorError> {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans");
    execute_statement(&plan, session, &EmptyProcedureRegistry)
}

fn assert_empty(output: StatementOutput) {
    assert!(matches!(output, StatementOutput::Empty));
}

#[test]
fn tx_op_start_with_no_active_txn_acquires_write_lock() {
    let graph = SharedGraph::new(GraphId::new(3821));
    let mut session = Session::new(&graph);

    assert_empty(execute("START TRANSACTION", &mut session).expect("start succeeds"));

    assert!(session.has_active_txn());
    session.abort();
}

#[test]
fn tx_op_start_with_active_txn_returns_already_active() {
    let graph = SharedGraph::new(GraphId::new(3822));
    let mut session = Session::new(&graph);
    execute("START TRANSACTION", &mut session).expect("start succeeds");

    let err = execute("START TRANSACTION", &mut session).expect_err("start errors");

    assert!(matches!(
        err,
        ExecutorError::TransactionAlreadyActive { .. }
    ));
    session.abort();
}

#[test]
fn tx_op_commit_with_active_txn_commits_and_clears() {
    let graph = SharedGraph::new(GraphId::new(3823));
    let mut session = Session::new(&graph);
    execute("START TRANSACTION", &mut session).expect("start succeeds");

    assert_empty(execute("COMMIT", &mut session).expect("commit succeeds"));

    assert!(!session.has_active_txn());
}

#[test]
fn tx_op_commit_with_no_active_txn_returns_no_active_transaction() {
    let graph = SharedGraph::new(GraphId::new(3824));
    let mut session = Session::new(&graph);

    let err = execute("COMMIT", &mut session).expect_err("commit errors");

    assert!(matches!(err, ExecutorError::NoActiveTransaction { .. }));
}

#[test]
fn tx_op_rollback_with_active_txn_drops_and_clears() {
    let graph = SharedGraph::new(GraphId::new(3825));
    let mut session = Session::new(&graph);
    execute("START TRANSACTION", &mut session).expect("start succeeds");

    assert_empty(execute("ROLLBACK", &mut session).expect("rollback succeeds"));

    assert!(!session.has_active_txn());
}

#[test]
fn tx_op_rollback_with_no_active_txn_returns_no_active_transaction() {
    let graph = SharedGraph::new(GraphId::new(3826));
    let mut session = Session::new(&graph);

    let err = execute("ROLLBACK", &mut session).expect_err("rollback errors");

    assert!(matches!(err, ExecutorError::NoActiveTransaction { .. }));
}

#[test]
fn tx_op_commit_accepts_session_principal() {
    let graph = SharedGraph::new(GraphId::new(3827));
    let principal = Arc::from([9_u8, 8, 7]);
    let mut session = Session::with_principal(&graph, principal);
    execute("START TRANSACTION", &mut session).expect("start succeeds");

    assert_empty(execute("COMMIT", &mut session).expect("commit succeeds"));

    assert!(!session.has_active_txn());
}

#[test]
fn tx_op_inside_execute_pipeline_returns_implementation_defined() {
    let graph = SharedGraph::new(GraphId::new(3828));
    let op = tx_op("START TRANSACTION");
    let caps = ImplDefinedCaps::default();
    let mut ctx = TxContext::read_only(
        graph.read(),
        &caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    );
    let input = BindingTable::new(
        BindingTableSchema {
            columns: Vec::new(),
        },
        vec![Binding::empty()],
    );

    let err = execute_pipeline(&[op], input, &mut ctx).expect_err("tx op is defensive error");

    assert!(matches!(
        err,
        ExecutorError::ImplementationDefined {
            detail: "TX op surfaced inside execute_pipeline; should be dispatched at statement level"
        }
    ));
}
