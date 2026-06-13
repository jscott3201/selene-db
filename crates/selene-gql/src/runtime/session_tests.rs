use std::{num::NonZeroUsize, time::Instant};

use selene_core::{BindingTableId, DbString, GraphId, Value, db_string};
use selene_graph::{GraphTypeDef, SharedGraph, TypedIndexKind};
use selene_persist::{DEFAULT_WAL_FILE_NAME, WalConfig};

use super::*;
use crate::{
    analyze::analyze,
    parser::parse,
    plan::plan,
    plan::{BindingTableSchema, ExecutionPlan},
    procedure_registry::EmptyProcedureRegistry,
    runtime::statement::{StatementOutput, execute_statement},
    runtime::{BindingTable, BindingTableRegistry},
};

fn planned(source: &str) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
}

fn execute(source: &str, session: &mut Session<'_>) -> Result<StatementOutput, ExecutorError> {
    let plan = planned(source);
    execute_statement(&plan, session, &EmptyProcedureRegistry)
}

fn admitted(value: &str) -> DbString {
    db_string(value).expect("test name admits")
}

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: admitted("Empty"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .expect("empty type validates")
        .build()
        .expect("closed graph builds")
}

fn empty_table() -> BindingTable {
    BindingTable::empty(BindingTableSchema {
        columns: Vec::new(),
    })
}

#[test]
fn table_parameter_replacements_share_scalar_namespace() {
    let graph = SharedGraph::new(GraphId::new(4000));
    let mut session = Session::new(&graph);
    let name = admitted("t");

    assert_eq!(session.bind_parameter(name.clone(), Value::Int(1)), None);
    assert!(matches!(
        session.bind_table_parameter(name.clone(), empty_table()),
        Some(SessionParameterValue::Scalar(Value::Int(1)))
    ));
    assert_eq!(session.bind_parameter(name.clone(), Value::Int(2)), None);
    assert_eq!(session.clear_parameter(&name), Some(Value::Int(2)));

    assert!(
        session
            .bind_table_parameter(name.clone(), empty_table())
            .is_none()
    );
    assert_eq!(session.clear_parameter(&name), None);
    assert!(session.parameters().is_empty());

    session.bind_parameter(name, Value::Int(3));
    session.clear_parameters();
    assert!(session.parameters().is_empty());
}

#[test]
fn scalar_only_materialization_borrows_parameter_map() {
    let graph = SharedGraph::new(GraphId::new(4003));
    let mut session = Session::new(&graph);
    let name = admitted("x");
    let registry = BindingTableRegistry::new();

    session.bind_parameter(name.clone(), Value::Int(7));

    let parameters = session.materialize_parameters(&registry);

    assert!(matches!(parameters, std::borrow::Cow::Borrowed(_)));
    assert_eq!(parameters.get(&name), Some(&Value::Int(7)));
}

#[test]
fn materialize_parameters_registers_table_values() {
    let graph = SharedGraph::new(GraphId::new(4001));
    let mut session = Session::new(&graph);
    let scalar = admitted("x");
    let table = admitted("t");
    let registry = BindingTableRegistry::new();

    session.bind_parameter(scalar.clone(), Value::Int(7));
    session.bind_table_parameter(table.clone(), empty_table());

    let parameters = session.materialize_parameters(&registry);

    assert!(matches!(parameters, std::borrow::Cow::Owned(_)));
    assert_eq!(parameters.get(&scalar), Some(&Value::Int(7)));
    let Some(Value::TableRef(id)) = parameters.get(&table) else {
        panic!("table parameter materializes as TableRef");
    };
    assert_ne!(*id, BindingTableId::TOMBSTONE);
    assert_eq!(
        registry
            .lookup(*id)
            .expect("materialized ID resolves")
            .row_count(),
        0
    );
}

#[test]
fn statement_execution_materializes_table_parameter_refs() {
    let graph = SharedGraph::new(GraphId::new(4002));
    let mut session = Session::new(&graph);
    session.bind_table_parameter(admitted("t"), empty_table());

    let output = execute("RETURN $t AS t", &mut session).expect("statement executes");
    let StatementOutput::Rows(table) = output else {
        panic!("RETURN should produce rows");
    };
    let Some(Value::TableRef(id)) = table.rows()[0].get(0) else {
        panic!("table parameter should surface as TableRef");
    };
    assert_ne!(*id, BindingTableId::TOMBSTONE);
}

#[test]
fn session_without_cache_executes_source_normally() {
    let graph = SharedGraph::new(GraphId::new(3897));
    let mut session = Session::new(&graph);

    let output = session
        .execute_source("RETURN 1", &EmptyProcedureRegistry)
        .expect("source executes");

    let StatementOutput::Rows(table) = output else {
        panic!("RETURN should produce rows");
    };
    assert_eq!(table.row_count(), 1);
    assert!(session.plan_cache_stats().is_none());
}

#[test]
fn session_with_cache_hits_on_second_source_execute() {
    let graph = SharedGraph::new(GraphId::new(3898));
    let mut session = Session::new(&graph).with_plan_cache(NonZeroUsize::new(4).expect("nonzero"));

    session
        .execute_source("RETURN 1", &EmptyProcedureRegistry)
        .expect("first source execute succeeds");
    session
        .execute_source("RETURN 1", &EmptyProcedureRegistry)
        .expect("second source execute succeeds");

    let stats = session.plan_cache_stats().expect("cache enabled");
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.stale_invalidations, 0);
}

#[test]
fn session_schema_change_invalidates_cached_source_plan() {
    let graph = SharedGraph::new(GraphId::new(3899));
    let mut session = Session::new(&graph).with_plan_cache(NonZeroUsize::new(4).expect("nonzero"));

    session
        .execute_source("RETURN 1", &EmptyProcedureRegistry)
        .expect("first source execute succeeds");
    graph
        .create_property_index(admitted("Person"), admitted("age"), TypedIndexKind::I64)
        .expect("index creation bumps schema");
    session
        .execute_source("RETURN 1", &EmptyProcedureRegistry)
        .expect("second source execute succeeds");

    let stats = session.plan_cache_stats().expect("cache enabled");
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.stale_invalidations, 1);
}

#[test]
fn execute_source_aborted_session_returns_in_failed_transaction_without_compiling() {
    // Codex PR #127 auto-review P2 #1: an aborted session must short-circuit
    // non-control source statements before analyze/plan/cache-insert work.
    let graph = SharedGraph::new(GraphId::new(3970));
    let mut session = Session::new(&graph);
    session.start_transaction().expect("start succeeds");
    session
        .execute_source(
            "INSERT (n:Person) SET n.age = 1 / 0 FINISH",
            &EmptyProcedureRegistry,
        )
        .expect_err("division-by-zero aborts the transaction");
    assert!(session.is_aborted());

    let err = session
        .execute_source("RETURN 1", &EmptyProcedureRegistry)
        .expect_err("aborted session rejects non-control");
    assert!(matches!(err, ExecutorError::InFailedTransaction { .. }));
    session.abort();
}

#[test]
fn execute_source_rollback_works_on_aborted_session() {
    // Aborted session must still accept TX-control statements (ROLLBACK
    // is the recovery path). Codex PR #127 auto-review P2 #1 contract.
    let graph = SharedGraph::new(GraphId::new(3971));
    let mut session = Session::new(&graph);
    session.start_transaction().expect("start succeeds");
    session
        .execute_source(
            "INSERT (n:Person) SET n.age = 1 / 0 FINISH",
            &EmptyProcedureRegistry,
        )
        .expect_err("division-by-zero aborts the transaction");
    assert!(session.is_aborted());

    session
        .execute_source("ROLLBACK", &EmptyProcedureRegistry)
        .expect("rollback succeeds on aborted session");
    assert!(!session.is_aborted());
    assert!(!session.has_active_txn());
}

#[test]
fn execute_source_parse_failure_aborts_active_tx() {
    // Codex PR #127 auto-review P2 #2: parse errors inside an explicit
    // transaction must abort the transaction (PostgreSQL-style "any error
    // → rollback" contract). Parallel for analyze/plan failures.
    let graph = SharedGraph::new(GraphId::new(3972));
    let mut session = Session::new(&graph);
    session.start_transaction().expect("start succeeds");

    let err = session
        .execute_source("NOT A VALID GQL STATEMENT", &EmptyProcedureRegistry)
        .expect_err("malformed source errors");
    assert!(matches!(err, ExecutorError::Parse { .. }));
    assert!(session.is_aborted());
    session.abort();
}

#[test]
fn execute_source_parse_failure_outside_tx_does_not_abort() {
    // Outside an explicit TX, the parse error returns as-is. Session has
    // no `aborted` semantics outside a TX (aborted flag is meaningless).
    let graph = SharedGraph::new(GraphId::new(3973));
    let mut session = Session::new(&graph);

    let err = session
        .execute_source("NOT A VALID GQL STATEMENT", &EmptyProcedureRegistry)
        .expect_err("malformed source errors");
    assert!(matches!(err, ExecutorError::Parse { .. }));
    assert!(!session.has_active_txn());
    assert!(!session.is_aborted());
}

#[test]
fn execute_source_uses_transaction_local_schema_after_catalog_change() {
    let graph = empty_closed_graph(3901);
    let mut session = Session::new(&graph).with_plan_cache(NonZeroUsize::new(4).expect("nonzero"));

    session
        .execute_source("START TRANSACTION", &EmptyProcedureRegistry)
        .expect("start succeeds");
    session
        .execute_source("CREATE NODE TYPE :Person ()", &EmptyProcedureRegistry)
        .expect("catalog source succeeds");
    session
        .execute_source("INSERT (:Person)", &EmptyProcedureRegistry)
        .expect("insert sees transaction-local schema");
    session
        .execute_source("COMMIT", &EmptyProcedureRegistry)
        .expect("commit succeeds");

    assert_eq!(graph.read().node_count(), 1);
    assert_eq!(
        graph.graph_type().expect("closed graph").node_types[0]
            .name
            .as_str(),
        "Person"
    );
}

#[test]
fn start_transaction_opens_active_txn() {
    let graph = SharedGraph::new(GraphId::new(3900));
    let mut session = Session::new(&graph);

    session.start_transaction().expect("start succeeds");

    assert!(session.has_active_txn());
    assert!(session.tx_started_at.is_some());
    assert_eq!(session.tx_statement_count, 0);
    session.abort();
}

#[test]
fn start_transaction_nested_errors() {
    let graph = SharedGraph::new(GraphId::new(3901));
    let mut session = Session::new(&graph);
    session.start_transaction().expect("start succeeds");

    let err = session
        .start_transaction()
        .expect_err("nested start errors");

    assert!(matches!(
        err,
        ExecutorError::TransactionAlreadyActive { .. }
    ));
    session.abort();
}

#[test]
fn commit_transaction_no_active_errors() {
    let graph = SharedGraph::new(GraphId::new(3902));
    let mut session = Session::new(&graph);

    let err = session
        .commit_transaction()
        .expect_err("commit without transaction errors");

    assert!(matches!(err, ExecutorError::NoActiveTransaction { .. }));
}

#[test]
fn rollback_transaction_no_active_errors() {
    let graph = SharedGraph::new(GraphId::new(3903));
    let mut session = Session::new(&graph);

    let err = session
        .rollback_transaction()
        .expect_err("rollback without transaction errors");

    assert!(matches!(err, ExecutorError::NoActiveTransaction { .. }));
}

#[test]
fn commit_aggregates_changes_across_statements() {
    let graph = SharedGraph::new(GraphId::new(3904));
    let mut session = Session::new(&graph);
    session.start_transaction().expect("start succeeds");

    execute("INSERT (:Person { name: 'a' })", &mut session).expect("first insert succeeds");
    execute("INSERT (:Person { name: 'b' })", &mut session).expect("second insert succeeds");
    execute("INSERT (:Person { name: 'c' })", &mut session).expect("third insert succeeds");
    let outcome = session.commit_transaction().expect("commit succeeds");

    assert_eq!(outcome.changes.len(), 3);
    assert_eq!(outcome.statement_count, 3);
    assert_eq!(graph.read().node_count(), 3);
}

#[test]
fn commit_counts_read_only_statement_inside_transaction() {
    let graph = SharedGraph::new(GraphId::new(3905));
    let mut session = Session::new(&graph);
    session.start_transaction().expect("start succeeds");

    execute("INSERT (:Person { name: 'a' })", &mut session).expect("insert succeeds");
    execute("MATCH (n:Person) RETURN n", &mut session).expect("read succeeds");
    let outcome = session.commit_transaction().expect("commit succeeds");

    assert_eq!(outcome.changes.len(), 1);
    assert_eq!(outcome.statement_count, 2);
}

#[test]
fn commit_returns_durable_at_with_core_provider() {
    let dir = tempfile::tempdir().expect("tempdir is created");
    let graph = SharedGraph::builder(GraphId::new(3906))
        .with_wal(dir.path().join(DEFAULT_WAL_FILE_NAME), WalConfig::default())
        .expect("wal config opens")
        .build()
        .expect("graph builds");
    let mut session = Session::new(&graph);
    session.start_transaction().expect("start succeeds");

    execute("INSERT (:Person { name: 'a' })", &mut session).expect("insert succeeds");
    let outcome = session.commit_transaction().expect("commit succeeds");

    assert_eq!(outcome.durable_at, Some(1));
}

#[test]
fn commit_returns_no_durable_at_without_provider() {
    let graph = SharedGraph::new(GraphId::new(3907));
    let mut session = Session::new(&graph);
    session.start_transaction().expect("start succeeds");

    execute("INSERT (:Person { name: 'a' })", &mut session).expect("insert succeeds");
    let outcome = session.commit_transaction().expect("commit succeeds");

    assert_eq!(outcome.durable_at, None);
}

#[test]
fn rollback_discards_changes() {
    let graph = SharedGraph::new(GraphId::new(3908));
    let mut session = Session::new(&graph);
    session.start_transaction().expect("start succeeds");

    execute("INSERT (:Person { name: 'a' })", &mut session).expect("insert succeeds");
    session.rollback_transaction().expect("rollback succeeds");

    assert_eq!(graph.read().node_count(), 0);
    assert!(!session.has_active_txn());
}

#[test]
fn rollback_outcome_reports_count() {
    let graph = SharedGraph::new(GraphId::new(3909));
    let mut session = Session::new(&graph);
    session.start_transaction().expect("start succeeds");

    execute("INSERT (:Person { name: 'a' })", &mut session).expect("first insert succeeds");
    execute("INSERT (:Person { name: 'b' })", &mut session).expect("second insert succeeds");
    let outcome = session.rollback_transaction().expect("rollback succeeds");

    assert_eq!(outcome.discarded_changes, 2);
    assert_eq!(outcome.statement_count, 2);
    assert_eq!(graph.read().node_count(), 0);
}

#[test]
fn duration_micros_is_populated() {
    let graph = SharedGraph::new(GraphId::new(3910));
    let mut session = Session::new(&graph);
    let started = Instant::now();
    session.start_transaction().expect("start succeeds");

    execute("INSERT (:Person { name: 'a' })", &mut session).expect("insert succeeds");
    let outcome = session.commit_transaction().expect("commit succeeds");

    assert!(outcome.duration_micros <= started.elapsed().as_micros() as u64);
}

#[test]
fn failed_statement_does_not_increment_statement_count() {
    // Codex auto-review P2: tx_statement_count only counts ACCEPTED
    // non-control statements. A statement that errors inside the
    // transaction window must not bump the counter — rollback's
    // RollbackOutcome.statement_count must match docs.
    let graph = SharedGraph::new(GraphId::new(3912));
    let mut session = Session::new(&graph);
    session.start_transaction().expect("start succeeds");

    execute("INSERT (:Person { name: 'a' })", &mut session).expect("first insert succeeds");
    execute("INSERT (n:Person) SET n.age = 1 / 0 FINISH", &mut session)
        .expect_err("division by zero aborts statement");
    let outcome = session.rollback_transaction().expect("rollback succeeds");

    // The first INSERT counts (1); the failed second statement does not.
    assert_eq!(outcome.statement_count, 1);
}

#[test]
fn abort_clears_tx_state_fields() {
    let graph = SharedGraph::new(GraphId::new(3911));
    let mut session = Session::new(&graph);
    session.start_transaction().expect("start succeeds");
    execute("INSERT (:Person { name: 'a' })", &mut session).expect("insert succeeds");

    assert!(session.tx_started_at.is_some());
    assert_eq!(session.tx_statement_count, 1);
    session.abort();

    assert!(!session.has_active_txn());
    assert!(session.tx_started_at.is_none());
    assert_eq!(session.tx_statement_count, 0);
    assert_eq!(graph.read().node_count(), 0);
}
