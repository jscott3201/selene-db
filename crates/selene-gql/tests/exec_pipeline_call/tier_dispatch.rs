//! Procedure CALL tier/transaction dispatch coverage — tier gating,
//! tx-state interactions, and dispatch error mapping, split out of the root
//! `exec_pipeline_call` binary to keep it under the repository 700-LOC cap.
//! Reuses the root binary's `TestRegistry` harness via `super::`.

use std::time::{Duration, Instant};

use selene_core::Value;
use selene_gql::{
    EmptyProcedureRegistry, ExecutorError, GqlStatus, GqlType, ProcedureError, ProcedureMutability,
    ProcedureTier, Session, StatementOutput, TxContext, execute_pipeline, execute_statement,
};

use super::{
    Behavior, TestRegistry, column_values, db_string, execute, execute_with_session, graph, output,
    param, planned, registry_one, rows, seed_table,
};

#[test]
fn unknown_procedure_at_runtime_maps_to_unknown_procedure_status() {
    let registry = registry_one(
        &["pkg", "rows"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("out", GqlType::Integer)],
        Behavior::Return(vec![vec![Value::Int(1)]]),
    );
    let plan = planned("CALL pkg.rows() YIELD out", &registry);
    let graph = graph(3900);
    let mut session = Session::new(&graph);

    let err = execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
        .expect_err("runtime registry misses handle");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::UnknownProcedure { .. },
            ..
        }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::UNKNOWN_PROCEDURE);
}

#[test]
fn read_tier_procedure_inside_write_tx_sees_pending_writes() {
    let registry = registry_one(
        &["pkg", "count"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("count", GqlType::Integer)],
        Behavior::CountNodes,
    );
    let graph = graph(3907);
    let mut session = Session::new(&graph);

    execute_with_session("START TRANSACTION", &mut session, &registry).unwrap();
    execute_with_session("INSERT (n:Person) FINISH", &mut session, &registry).unwrap();
    let table = rows(
        execute_with_session("CALL pkg.count() YIELD count", &mut session, &registry).unwrap(),
    );

    assert_eq!(column_values(&table, "count"), vec![Value::Int(1)]);
    assert_eq!(graph.read().node_count(), 0);
    session.abort();
}

#[test]
fn mutation_tier_procedure_in_auto_commit_commits_on_success() {
    let registry = registry_one(
        &["pkg", "create"],
        ProcedureMutability::SchemaWrite,
        ProcedureTier::Mutation,
        Vec::new(),
        Behavior::CreateNode(db_string("FromProc")),
    );
    let graph = graph(3908);

    let output = execute("CALL pkg.create()", &graph, &registry).unwrap();

    assert!(matches!(
        output,
        StatementOutput::Written(outcome) if outcome.changes.len() == 1
    ));
    assert_eq!(graph.read().node_count(), 1);
    assert_eq!(registry.records()[0].tier, ProcedureTier::Mutation);
}

#[test]
fn maintenance_tier_procedure_runs_without_write_commit() {
    let registry = registry_one(
        &["pkg", "maintain"],
        ProcedureMutability::MaintenanceWrite,
        ProcedureTier::Maintenance,
        vec![output("out", GqlType::Integer)],
        Behavior::Return(vec![vec![Value::Int(11)]]),
    );
    let graph = graph(3926);

    let table = rows(execute("CALL pkg.maintain() YIELD out", &graph, &registry).unwrap());

    assert_eq!(column_values(&table, "out"), vec![Value::Int(11)]);
    assert_eq!(graph.read().node_count(), 0);
    assert_eq!(registry.records()[0].tier, ProcedureTier::Maintenance);
}

#[test]
fn maintenance_tier_procedure_in_explicit_tx_is_rejected() {
    let registry = registry_one(
        &["pkg", "maintain"],
        ProcedureMutability::MaintenanceWrite,
        ProcedureTier::Maintenance,
        Vec::new(),
        Behavior::Return(vec![vec![]]),
    );
    let graph = graph(3927);
    let mut session = Session::new(&graph);

    execute_with_session("START TRANSACTION", &mut session, &registry).unwrap();
    let err = execute_with_session("CALL pkg.maintain()", &mut session, &registry)
        .expect_err("maintenance rejects explicit transaction");

    assert!(matches!(
        err,
        ExecutorError::InvalidTransactionState {
            detail: "maintenance procedure cannot run inside an explicit transaction",
            ..
        }
    ));
    assert!(registry.records().is_empty());
    assert!(session.is_aborted());
    session.abort();
}

#[test]
fn maintenance_tier_procedure_in_plain_read_context_is_rejected() {
    let registry = registry_one(
        &["pkg", "maintain"],
        ProcedureMutability::MaintenanceWrite,
        ProcedureTier::Maintenance,
        Vec::new(),
        Behavior::Return(vec![vec![]]),
    );
    let plan = planned("CALL pkg.maintain()", &registry);
    let graph = graph(3928);
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &registry,
        graph.index_providers(),
    );

    let err = execute_pipeline(&plan.pipeline, seed_table(), &mut ctx)
        .expect_err("maintenance context is absent");

    assert!(matches!(
        err,
        ExecutorError::InvalidTransactionState {
            detail: "maintenance-tier procedure requires a maintenance statement context",
            ..
        }
    ));
}

#[test]
fn mutation_tier_procedure_in_explicit_tx_sees_pending_state() {
    let registry = registry_one(
        &["pkg", "count"],
        ProcedureMutability::SchemaWrite,
        ProcedureTier::Mutation,
        vec![output("count", GqlType::Integer)],
        Behavior::CountNodes,
    );
    let graph = graph(3909);
    let mut session = Session::new(&graph);

    execute_with_session("START TRANSACTION", &mut session, &registry).unwrap();
    execute_with_session("INSERT (n:Person) FINISH", &mut session, &registry).unwrap();
    let table = rows(
        execute_with_session("CALL pkg.count() YIELD count", &mut session, &registry).unwrap(),
    );

    assert_eq!(column_values(&table, "count"), vec![Value::Int(1)]);
    assert_eq!(graph.read().node_count(), 0);
    session.abort();
}

#[test]
fn mutation_tier_procedure_in_read_only_context_returns_invalid_transaction_state() {
    let registry = registry_one(
        &["pkg", "create"],
        ProcedureMutability::SchemaWrite,
        ProcedureTier::Mutation,
        Vec::new(),
        Behavior::CreateNode(db_string("FromProc")),
    );
    let plan = planned("CALL pkg.create()", &registry);
    let graph = graph(3910);
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &registry,
        graph.index_providers(),
    );

    let err =
        execute_pipeline(&plan.pipeline, seed_table(), &mut ctx).expect_err("needs write txn");

    assert!(matches!(
        err,
        ExecutorError::InvalidTransactionState {
            detail: "mutation-tier procedure requires a write transaction",
            ..
        }
    ));
}

#[test]
fn mutation_tier_procedure_failure_inside_explicit_tx_marks_session_aborted() {
    let registry = registry_one(
        &["pkg", "fail"],
        ProcedureMutability::SchemaWrite,
        ProcedureTier::Mutation,
        Vec::new(),
        Behavior::Error(ProcedureError::InvalidArgument {
            detail: "bad input".to_owned(),
        }),
    );
    let graph = graph(3911);
    let mut session = Session::new(&graph);

    execute_with_session("START TRANSACTION", &mut session, &registry).unwrap();
    let err = execute_with_session("CALL pkg.fail()", &mut session, &registry)
        .expect_err("procedure fails");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
    assert!(session.is_aborted());
    session.abort();
}

#[test]
fn tier_mismatch_between_metadata_and_dispatch_returns_tier_mismatch() {
    let registry = registry_one(
        &["pkg", "bad"],
        ProcedureMutability::Read,
        ProcedureTier::Mutation,
        Vec::new(),
        Behavior::Return(vec![vec![]]),
    );

    let err = execute("CALL pkg.bad()", &graph(3912), &registry).expect_err("tier mismatch");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::TierMismatch {
                expected: ProcedureTier::Graph,
                actual: ProcedureTier::Mutation
            },
            ..
        }
    ));
}

#[test]
fn procedure_returns_wrong_column_count_returns_internal_error() {
    let registry = registry_one(
        &["pkg", "bad"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("out", GqlType::Integer)],
        Behavior::Return(vec![vec![]]),
    );

    let err = execute("CALL pkg.bad() YIELD out", &graph(3913), &registry)
        .expect_err("row width mismatch");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::Internal { detail },
            ..
        } if detail == "registry returned row with wrong column count"
    ));
}

#[test]
fn procedure_returns_wrong_value_type_returns_internal_error() {
    let registry = registry_one(
        &["pkg", "bad"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("out", GqlType::Integer)],
        Behavior::Return(vec![vec![Value::String(db_string("wrong"))]]),
    );

    let err = execute("CALL pkg.bad() YIELD out", &graph(3914), &registry)
        .expect_err("row type mismatch");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::Internal { detail },
            ..
        } if detail == "registry returned value with wrong type for column 0"
    ));
}

#[test]
fn procedure_timeout_preserves_session_deadline() {
    let elapsed = Duration::from_millis(7);
    let registry = registry_one(
        &["pkg", "timeout"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        Vec::new(),
        Behavior::Error(ProcedureError::Timeout { elapsed }),
    );
    let graph = graph(3915);
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut session = Session::new(&graph).with_deadline(deadline);

    let err = execute_with_session("CALL pkg.timeout()", &mut session, &registry)
        .expect_err("procedure reports timeout");

    let ExecutorError::Timeout {
        deadline: observed,
        elapsed: observed_elapsed,
        ..
    } = err
    else {
        panic!("expected timeout, got {err:?}");
    };
    assert_eq!(observed, deadline);
    assert_eq!(observed_elapsed, elapsed);
}

#[test]
fn arg_evaluation_failure_propagates_without_dispatch() {
    let registry = TestRegistry::new().with_procedure(
        &["pkg", "echo"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![param("value", GqlType::Integer, false)],
        vec![output("out", GqlType::Integer)],
        Behavior::Return(vec![vec![Value::Int(1)]]),
    );

    let err = execute("CALL pkg.echo(1 / 0) YIELD out", &graph(3916), &registry)
        .expect_err("arg evaluation fails");

    assert!(matches!(err, ExecutorError::DataException { .. }));
    assert!(registry.records().is_empty());
}

#[test]
fn procedure_with_zero_args_dispatches_with_empty_arg_slice() {
    let registry = registry_one(
        &["pkg", "unit"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        Vec::new(),
        Behavior::Return(vec![vec![]]),
    );

    execute("CALL pkg.unit()", &graph(3917), &registry).unwrap();

    assert_eq!(registry.records()[0].args, Vec::<Value>::new());
}
