//! BRIEF-38 statement-level executor tests.

use selene_core::{GraphId, LabelSet, Value, intern};
use selene_gql::{
    EmptyProcedureRegistry, ExecutionPlan, ExecutorError, GqlStatus, Session, StatementOutput,
    analyze, execute_statement, parse, plan,
};
use selene_graph::{GraphTypeDef, NodeTypeDef, SharedGraph};

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
fn data_modifying_auto_commits_insert_and_returns_empty_for_finish() {
    let graph = SharedGraph::new(GraphId::new(3802));
    let mut session = Session::new(&graph);

    let output = execute("INSERT (n:Person) FINISH", &mut session).expect("insert executes");

    assert!(matches!(output, StatementOutput::Empty));
    assert_eq!(graph.read().node_count(), 1);
}

#[test]
fn mutation_with_return_yields_rows() {
    let graph = SharedGraph::new(GraphId::new(3803));
    let mut session = Session::new(&graph);

    let table = rows(execute("INSERT (n:Person) RETURN n", &mut session).expect("insert executes"));

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

    assert!(matches!(output, StatementOutput::Empty));
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
    execute("COMMIT", &mut session).expect("commit succeeds");

    assert!(!session.has_active_txn());
    assert_eq!(graph.read().node_count(), 1);
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
fn explicit_tx_error_leaves_session_active_until_abort() {
    let graph = SharedGraph::new(GraphId::new(3809));
    let mut session = Session::new(&graph);

    execute("START TRANSACTION", &mut session).expect("start succeeds");
    let err = execute("INSERT (n:Person) SET n.age = 1 / 0 FINISH", &mut session)
        .expect_err("insert errors");

    assert!(matches!(err, ExecutorError::DataException { .. }));
    assert!(session.has_active_txn());
    assert_eq!(graph.read().node_count(), 0);
    session.abort();
    assert!(!session.has_active_txn());
    assert_eq!(graph.read().node_count(), 0);
}

#[test]
fn read_only_plan_during_active_tx_uses_fresh_snapshot() {
    let graph = SharedGraph::new(GraphId::new(3810));
    let mut session = Session::new(&graph);

    execute("START TRANSACTION", &mut session).expect("start succeeds");
    execute("INSERT (n:Person) FINISH", &mut session).expect("insert succeeds");
    let table = rows(execute("MATCH (n:Person) RETURN n", &mut session).expect("read executes"));

    assert_eq!(table.row_count(), 0);
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
