//! End-to-end coverage for rank-fusion `selene.*` built-ins.

use selene_core::{DbString, GraphId, NodeId, Value};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ExecutorError, ProcedureError, ProcedureRegistry,
    Session, StatementOutput,
};
use selene_graph::SharedGraph;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("written statement returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn execute_rows(
    session: &mut Session<'_>,
    source: &str,
    registry: &dyn ProcedureRegistry,
) -> BindingTable {
    rows(
        session
            .execute_source(source, registry)
            .expect("statement executes"),
    )
}

fn node_list(ids: &[u64]) -> Value {
    Value::List(
        ids.iter()
            .copied()
            .map(|id| Value::NodeRef(NodeId::new(id)))
            .collect(),
    )
}

fn rankings(lists: &[&[u64]]) -> Value {
    Value::List(lists.iter().map(|ids| node_list(ids)).collect())
}

fn weights(values: &[f64]) -> Value {
    Value::List(values.iter().copied().map(Value::Float).collect())
}

fn node_column(table: &BindingTable, name: &str) -> Vec<NodeId> {
    let index = table
        .column_index(db_string(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::NodeRef(value)) => *value,
            other => panic!("expected node ref in {name}, got {other:?}"),
        })
        .collect()
}

fn float_column(table: &BindingTable, name: &str) -> Vec<f64> {
    let index = table
        .column_index(db_string(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::Float(value)) => *value,
            other => panic!("expected float in {name}, got {other:?}"),
        })
        .collect()
}

fn invalid_arg_detail(error: ExecutorError) -> String {
    match error {
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { detail },
            ..
        } => detail,
        other => panic!("expected invalid procedure argument, got {other:?}"),
    }
}

#[test]
fn reciprocal_rank_fusion_returns_overlap_first() {
    let graph = graph(330_701);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("rankings"), rankings(&[&[1, 2], &[2, 3]]));

    let table = execute_rows(
        &mut session,
        "CALL selene.reciprocal_rank_fusion($rankings, 3) YIELD node_id, score",
        &registry,
    );

    assert_eq!(
        node_column(&table, "node_id"),
        vec![NodeId::new(2), NodeId::new(1), NodeId::new(3)]
    );
    let scores = float_column(&table, "score");
    assert!(scores[0] > scores[1]);
    assert!(scores[1] > scores[2]);
}

#[test]
fn reciprocal_rank_fusion_applies_weights_and_rank_constant() {
    let graph = graph(330_702);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("rankings"), rankings(&[&[1, 2], &[2, 3]]));
    session.bind_parameter(db_string("weights"), weights(&[0.0, 2.0]));

    let table = execute_rows(
        &mut session,
        "CALL selene.reciprocal_rank_fusion($rankings, 3, 10.0, $weights) \
         YIELD node_id, score",
        &registry,
    );

    assert_eq!(
        node_column(&table, "node_id"),
        vec![NodeId::new(2), NodeId::new(3)]
    );
    let scores = float_column(&table, "score");
    assert!((scores[0] - (2.0 / 11.0)).abs() < 1e-12);
    assert!((scores[1] - (2.0 / 12.0)).abs() < 1e-12);
}

#[test]
fn reciprocal_rank_fusion_rejects_invalid_arguments() {
    let graph = graph(330_703);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("rankings"), rankings(&[&[1, 2], &[2, 3]]));
    session.bind_parameter(db_string("weights"), weights(&[1.0]));
    let err = session
        .execute_source(
            "CALL selene.reciprocal_rank_fusion($rankings, 3, 60, $weights)",
            &registry,
        )
        .expect_err("mismatched weights reject");
    assert!(invalid_arg_detail(err).contains("weights length 1 must match rankings length 2"));

    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("rankings"), rankings(&[&[1]]));
    let err = session
        .execute_source(
            "CALL selene.reciprocal_rank_fusion($rankings, 3, 0.0)",
            &registry,
        )
        .expect_err("zero rank constant rejects");
    assert!(invalid_arg_detail(err).contains("rank_constant must be a positive finite"));

    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("rankings"), rankings(&[&[1]]));
    let err = session
        .execute_source(
            "CALL selene.reciprocal_rank_fusion($rankings, -1)",
            &registry,
        )
        .expect_err("negative k rejects");
    assert!(invalid_arg_detail(err).contains("k must be a non-negative INTEGER"));

    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("rankings"),
        Value::List(vec![Value::List(vec![
            Value::NodeRef(NodeId::new(1)),
            Value::Int(7),
        ])]),
    );
    let err = session
        .execute_source(
            "CALL selene.reciprocal_rank_fusion($rankings, 3)",
            &registry,
        )
        .expect_err("non-node ranking entry rejects");
    assert!(invalid_arg_detail(err).contains("rankings[0][1] must be a NODE"));
}
