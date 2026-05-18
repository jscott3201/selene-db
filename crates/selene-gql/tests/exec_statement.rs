//! BRIEF-38 statement-level executor tests.

use std::sync::Arc;

use selene_core::{GraphId, LabelSet, Value, intern};
use selene_gql::{
    EmptyProcedureRegistry, ExecutionPlan, ExecutorError, GqlStatus, Session, StatementOutput,
    WriteOutcome, analyze, execute_statement, parse, plan,
};
use selene_graph::{GraphTypeDef, NodeTypeDef, SharedGraph};
use selene_persist::{DEFAULT_WAL_FILE_NAME, WalConfig};

fn istr(value: &str) -> selene_core::IStr {
    intern(value).expect("test string interns")
}

fn planned(source: &str) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
}

fn execute(source: &str, session: &mut Session<'_>) -> Result<StatementOutput, ExecutorError> {
    let plan = planned(source);
    execute_statement(&plan, session, &EmptyProcedureRegistry)
}

fn rows(output: StatementOutput) -> selene_gql::BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        other => panic!("expected rows, got {other:?}"),
    }
}

fn written(output: StatementOutput) -> WriteOutcome {
    match output {
        StatementOutput::Written(outcome) => outcome,
        other => panic!("expected written output, got {other:?}"),
    }
}

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: istr("statement.test.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

fn closed_person_graph(id: u64) -> SharedGraph {
    let person = istr("Person");
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: istr("statement.person.graph"),
            node_types: vec![NodeTypeDef {
                name: person,
                key_labels: LabelSet::single(person),
                properties: Vec::new(),
            }],
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

#[test]
fn read_only_returns_rows_from_pattern_less_plan() {
    let graph = SharedGraph::new(GraphId::new(3800));
    let mut session = Session::new(&graph);

    let table = rows(execute("RETURN 1 AS n", &mut session).expect("statement executes"));

    assert_eq!(table.row_count(), 1);
    assert_eq!(table.rows()[0].values(), &[Value::Int(1)]);
}

#[test]
fn read_only_returns_rows_from_pattern_plan() {
    let graph = SharedGraph::new(GraphId::new(3801));
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(istr("Person")), Default::default())
            .expect("fixture node inserts");
        txn.commit().expect("fixture commits");
    }
    let mut session = Session::new(&graph);

    let table =
        rows(execute("MATCH (n:Person) RETURN n", &mut session).expect("statement executes"));

    assert_eq!(table.row_count(), 1);
    assert!(matches!(table.rows()[0].values(), [Value::NodeRef(_)]));
}

#[test]
fn write_statement_produces_written() {
    let graph = SharedGraph::new(GraphId::new(3802));
    let mut session = Session::new(&graph);

    let output = execute("INSERT (n:Person) FINISH", &mut session).expect("insert executes");
    let outcome = written(output);

    assert!(outcome.rows.is_none());
    assert_eq!(outcome.changes.len(), 1);
    assert_eq!(outcome.durable_at, None);
    assert_eq!(graph.read().node_count(), 1);
}

#[test]
fn mutation_with_return_carries_rows() {
    let graph = SharedGraph::new(GraphId::new(3803));
    let mut session = Session::new(&graph);

    let outcome =
        written(execute("INSERT (n:Person) RETURN n", &mut session).expect("insert executes"));
    let table = outcome.rows.expect("write with RETURN carries rows");

    assert_eq!(outcome.changes.len(), 1);
    assert_eq!(table.row_count(), 1);
    assert!(matches!(table.rows()[0].values(), [Value::NodeRef(_)]));
    assert_eq!(graph.read().node_count(), 1);
}

#[test]
fn data_modifying_auto_rolls_back_on_error() {
    let graph = SharedGraph::new(GraphId::new(3804));
    let mut session = Session::new(&graph);

    let err = execute("INSERT (n:Person) SET n.age = 1 / 0 FINISH", &mut session)
        .expect_err("statement errors");

    assert!(matches!(err, ExecutorError::DataException { .. }));
    assert_eq!(graph.read().node_count(), 0);
}

#[test]
fn catalog_modifying_auto_commits_create_type() {
    let graph = empty_closed_graph(3805);
    let mut session = Session::new(&graph);

    let output = execute("CREATE NODE TYPE :Foo ()", &mut session).expect("catalog executes");
    let outcome = written(output);

    assert!(outcome.rows.is_none());
    assert_eq!(outcome.changes.len(), 1);
    let graph_type = graph.graph_type().expect("closed graph type");
    assert_eq!(graph_type.node_types[0].name.as_str(), "Foo");
}

#[test]
fn catalog_show_yields_rows() {
    let graph = SharedGraph::builder(GraphId::new(3806))
        .bound_to(GraphTypeDef {
            name: istr("statement.show.graph"),
            node_types: vec![NodeTypeDef {
                name: istr("types.person"),
                key_labels: LabelSet::single(istr("Person")),
                properties: Vec::new(),
            }],
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap();
    let mut session = Session::new(&graph);

    let table = rows(execute("SHOW NODE TYPES", &mut session).expect("show executes"));

    assert_eq!(table.row_count(), 1);
    assert_eq!(table.rows()[0].values()[0], Value::String(istr("Person")));
}

#[test]
fn explicit_tx_commit_persists_changes() {
    let graph = SharedGraph::new(GraphId::new(3807));
    let mut session = Session::new(&graph);

    assert!(matches!(
        execute("START TRANSACTION", &mut session).expect("start succeeds"),
        StatementOutput::Empty
    ));
    assert!(session.has_active_txn());
    execute("INSERT (n:Person) FINISH", &mut session).expect("insert succeeds");
    assert_eq!(graph.read().node_count(), 0);
    let outcome = written(execute("COMMIT", &mut session).expect("commit succeeds"));

    assert_eq!(outcome.changes.len(), 1);
    assert!(!session.has_active_txn());
    assert_eq!(graph.read().node_count(), 1);
}

#[test]
fn commit_with_zero_changes_returns_written() {
    let graph = SharedGraph::new(GraphId::new(3819));
    let mut session = Session::new(&graph);

    execute("START TRANSACTION", &mut session).expect("start succeeds");
    let outcome = written(execute("COMMIT", &mut session).expect("commit succeeds"));

    assert!(outcome.rows.is_none());
    assert!(outcome.changes.is_empty());
    assert_eq!(outcome.generation, 1);
    assert!(!session.has_active_txn());
}

#[test]
fn write_outcome_durable_at_some_for_with_wal_and_flush_returns_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let graph = SharedGraph::builder(GraphId::new(3820))
        .with_wal(dir.path().join(DEFAULT_WAL_FILE_NAME), WalConfig::default())
        .unwrap()
        .build()
        .unwrap();
    let mut session = Session::new(&graph);

    let outcome =
        written(execute("INSERT (n:Person) FINISH", &mut session).expect("insert executes"));

    assert_eq!(outcome.durable_at, Some(1));
    assert_eq!(session.flush().expect("flush succeeds"), Some(1));
}

#[test]
fn explicit_tx_rollback_discards_changes() {
    let graph = SharedGraph::new(GraphId::new(3808));
    let mut session = Session::new(&graph);

    execute("START TRANSACTION", &mut session).expect("start succeeds");
    execute("INSERT (n:Person) FINISH", &mut session).expect("insert succeeds");
    execute("ROLLBACK", &mut session).expect("rollback succeeds");

    assert!(!session.has_active_txn());
    assert_eq!(graph.read().node_count(), 0);
}

#[test]
fn explicit_tx_statement_error_aborts_session() {
    let graph = SharedGraph::new(GraphId::new(3809));
    let mut session = Session::new(&graph);

    execute("START TRANSACTION", &mut session).expect("start succeeds");
    let err = execute("INSERT (n:Person) SET n.age = 1 / 0 FINISH", &mut session)
        .expect_err("insert errors");

    assert!(matches!(err, ExecutorError::DataException { .. }));
    assert!(session.has_active_txn());
    assert!(session.is_aborted());
    assert_eq!(graph.read().node_count(), 0);
    session.abort();
    assert!(!session.has_active_txn());
    assert!(!session.is_aborted());
    assert_eq!(graph.read().node_count(), 0);
}

#[test]
fn read_only_during_active_tx_sees_uncommitted_writes() {
    let graph = SharedGraph::new(GraphId::new(3810));
    let mut session = Session::new(&graph);

    execute("START TRANSACTION", &mut session).expect("start succeeds");
    execute("INSERT (n:Person) FINISH", &mut session).expect("insert succeeds");
    let table = rows(execute("MATCH (n:Person) RETURN n", &mut session).expect("read executes"));

    assert_eq!(table.row_count(), 1);
    assert!(session.has_active_txn());
    session.abort();
}

#[test]
fn start_transaction_with_active_txn_returns_already_active() {
    let graph = SharedGraph::new(GraphId::new(3811));
    let mut session = Session::new(&graph);
    execute("START TRANSACTION", &mut session).expect("start succeeds");

    let err = execute("START TRANSACTION", &mut session).expect_err("second start errors");

    assert!(matches!(
        err,
        ExecutorError::TransactionAlreadyActive { .. }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::INVALID_TRANSACTION_STATE);
    session.abort();
}

#[test]
fn commit_without_active_txn_returns_no_active_transaction() {
    let graph = SharedGraph::new(GraphId::new(3812));
    let mut session = Session::new(&graph);

    let err = execute("COMMIT", &mut session).expect_err("commit errors");

    assert!(matches!(err, ExecutorError::NoActiveTransaction { .. }));
    assert_eq!(err.gqlstatus(), GqlStatus::INVALID_TRANSACTION_STATE);
}

#[test]
fn rollback_without_active_txn_returns_no_active_transaction() {
    let graph = SharedGraph::new(GraphId::new(3813));
    let mut session = Session::new(&graph);

    let err = execute("ROLLBACK", &mut session).expect_err("rollback errors");

    assert!(matches!(err, ExecutorError::NoActiveTransaction { .. }));
    assert_eq!(err.gqlstatus(), GqlStatus::INVALID_TRANSACTION_STATE);
}

#[test]
fn aborted_session_rejects_data_modifying_with_in_failed_transaction() {
    let graph = SharedGraph::new(GraphId::new(3814));
    let mut session = Session::new(&graph);
    execute("START TRANSACTION", &mut session).expect("start succeeds");
    execute("INSERT (n:Person) SET n.age = 1 / 0 FINISH", &mut session)
        .expect_err("statement aborts transaction");

    let err = execute("INSERT (n:Other) FINISH", &mut session).expect_err("aborted tx rejects");

    assert!(matches!(err, ExecutorError::InFailedTransaction { .. }));
    assert_eq!(err.gqlstatus(), GqlStatus::INVALID_TRANSACTION_STATE);
    assert!(session.is_aborted());
    session.abort();
}

#[test]
fn aborted_session_rejects_read_only_with_in_failed_transaction() {
    let graph = SharedGraph::new(GraphId::new(3815));
    let mut session = Session::new(&graph);
    execute("START TRANSACTION", &mut session).expect("start succeeds");
    execute("INSERT (n:Person) SET n.age = 1 / 0 FINISH", &mut session)
        .expect_err("statement aborts transaction");

    let err = execute("RETURN 1 AS n", &mut session).expect_err("aborted tx rejects");

    assert!(matches!(err, ExecutorError::InFailedTransaction { .. }));
    assert!(session.is_aborted());
    session.abort();
}

#[test]
fn aborted_session_commit_rolls_back_and_returns_in_failed_transaction() {
    let graph = SharedGraph::new(GraphId::new(3816));
    let mut session = Session::new(&graph);
    execute("START TRANSACTION", &mut session).expect("start succeeds");
    execute("INSERT (n:Person) SET n.age = 1 / 0 FINISH", &mut session)
        .expect_err("statement aborts transaction");

    let err = execute("COMMIT", &mut session).expect_err("commit refuses aborted tx");

    assert!(matches!(err, ExecutorError::InFailedTransaction { .. }));
    assert!(!session.has_active_txn());
    assert!(!session.is_aborted());
    assert_eq!(graph.read().node_count(), 0);
}

#[test]
fn aborted_session_rollback_clears_aborted_state() {
    let graph = SharedGraph::new(GraphId::new(3817));
    let mut session = Session::new(&graph);
    execute("START TRANSACTION", &mut session).expect("start succeeds");
    execute("INSERT (n:Person) SET n.age = 1 / 0 FINISH", &mut session)
        .expect_err("statement aborts transaction");

    execute("ROLLBACK", &mut session).expect("rollback succeeds");

    assert!(!session.has_active_txn());
    assert!(!session.is_aborted());
    assert_eq!(graph.read().node_count(), 0);
}

#[test]
fn aborted_session_abort_helper_clears_aborted_state() {
    let graph = SharedGraph::new(GraphId::new(3818));
    let mut session = Session::new(&graph);
    execute("START TRANSACTION", &mut session).expect("start succeeds");
    execute("INSERT (n:Person) SET n.age = 1 / 0 FINISH", &mut session)
        .expect_err("statement aborts transaction");

    session.abort();

    assert!(!session.has_active_txn());
    assert!(!session.is_aborted());
    assert_eq!(graph.read().node_count(), 0);
}

#[test]
fn partial_writes_from_failed_statement_do_not_persist_after_rollback() {
    let graph = SharedGraph::new(GraphId::new(3819));
    let mut session = Session::new(&graph);
    execute("START TRANSACTION", &mut session).expect("start succeeds");

    execute("INSERT (n:Person) SET n.age = 1 / 0 FINISH", &mut session)
        .expect_err("statement aborts transaction");
    execute("ROLLBACK", &mut session).expect("rollback succeeds");

    assert_eq!(graph.read().node_count(), 0);
}

#[test]
fn multi_statement_explicit_tx_commits_all_changes() {
    let graph = SharedGraph::new(GraphId::new(3820));
    let mut session = Session::new(&graph);

    execute("START TRANSACTION", &mut session).expect("start succeeds");
    execute("INSERT (n:Person) FINISH", &mut session).expect("first insert succeeds");
    execute("INSERT (n:Company) FINISH", &mut session).expect("second insert succeeds");
    execute("COMMIT", &mut session).expect("commit succeeds");

    assert_eq!(graph.read().node_count(), 2);
}

#[test]
fn session_abort_clears_active_txn() {
    let graph = SharedGraph::new(GraphId::new(3821));
    let mut session = Session::new(&graph);

    execute("START TRANSACTION", &mut session).expect("start succeeds");
    session.abort();

    assert!(!session.has_active_txn());
    assert!(!session.is_aborted());
}

#[test]
fn session_with_principal_commits_successfully() {
    let graph = SharedGraph::new(GraphId::new(3822));
    let principal = Arc::from([1_u8, 2, 3]);
    let mut session = Session::with_principal(&graph, principal);

    execute("START TRANSACTION", &mut session).expect("start succeeds");
    execute("INSERT (n:Person) FINISH", &mut session).expect("insert succeeds");
    execute("COMMIT", &mut session).expect("commit succeeds");

    assert_eq!(graph.read().node_count(), 1);
}

#[test]
fn catalog_modifying_auto_rolls_back_on_validation_error() {
    let graph = closed_person_graph(3823);
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(istr("Person")), Default::default())
            .expect("fixture node inserts");
        txn.commit().expect("fixture commits");
    }
    let mut session = Session::new(&graph);

    let err = execute("DROP NODE TYPE :Person", &mut session).expect_err("commit rejects drop");

    assert!(matches!(err, ExecutorError::GraphMutation { .. }));
    assert_eq!(graph.graph_type().unwrap().node_types.len(), 1);
    assert_eq!(graph.read().node_count(), 1);
}

#[test]
fn explicit_tx_allows_data_then_catalog_mix_in_phase_a() {
    let graph = empty_closed_graph(3824);
    let mut session = Session::new(&graph);

    execute("START TRANSACTION", &mut session).expect("start succeeds");
    execute("INSERT (n:Person) FINISH", &mut session).expect("insert succeeds");
    execute("CREATE NODE TYPE :Person ()", &mut session).expect("catalog succeeds");
    execute("COMMIT", &mut session).expect("commit succeeds");

    assert_eq!(graph.read().node_count(), 1);
    assert_eq!(
        graph.graph_type().unwrap().node_types[0].name.as_str(),
        "Person"
    );
}
