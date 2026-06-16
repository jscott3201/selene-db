//! BRIEF-127 set-operation runtime coverage.

mod exec_common;

use std::num::NonZeroUsize;

use exec_common::{column_values, execute_read, planned};
use selene_core::{CancellationToken, GraphId, Value, db_string};
use selene_gql::{
    AnalyzedType, Binding, BindingTable, BindingTableColumn, BindingTableSchema,
    DataExceptionSubclass, EmptyProcedureRegistry, ExecutorError, GqlType, ImplDefinedCaps,
    PipelineOp, Session, SetOp, StatementOutput, TxContext, execute_pipeline,
};
use selene_graph::SharedGraph;

fn int_table(name: &str, values: impl IntoIterator<Item = i64>) -> BindingTable {
    BindingTable::new(
        BindingTableSchema {
            columns: vec![BindingTableColumn {
                name: Some(exec_common::db_string(name)),
                hidden: None,
                ty: AnalyzedType::Resolved(GqlType::Integer),
            }],
        },
        values
            .into_iter()
            .map(|value| Binding::new([Value::Int(value)]))
            .collect(),
    )
}

fn string_table(name: &str, values: Vec<Value>) -> BindingTable {
    BindingTable::new(
        BindingTableSchema {
            columns: vec![BindingTableColumn {
                name: Some(exec_common::db_string(name)),
                hidden: None,
                ty: AnalyzedType::Resolved(GqlType::String),
            }],
        },
        values
            .into_iter()
            .map(|value| Binding::new([value]))
            .collect(),
    )
}

fn execute_manual_set_op(
    op: SetOp,
    rhs_source: &str,
    input: BindingTable,
    caps: &ImplDefinedCaps,
    cancellation: Option<&CancellationToken>,
) -> Result<BindingTable, ExecutorError> {
    let rhs = planned(rhs_source);
    let graph = SharedGraph::new(GraphId::new(7127));
    let mut ctx = TxContext::read_only(
        graph.read(),
        caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    )
    .with_resource_limits(cancellation, None, None, None);
    execute_pipeline(
        &[PipelineOp::Union {
            op,
            rhs: Box::new(rhs),
        }],
        input,
        &mut ctx,
    )
}

#[test]
fn intersect_returns_lhs_input_order() {
    let table = execute_read("FOR n IN [1, 2, 3] RETURN n INTERSECT FOR n IN [2, 3, 4] RETURN n");

    assert_eq!(
        column_values(&table, "n"),
        vec![Value::Int(2), Value::Int(3)]
    );
}

#[test]
fn intersect_all_returns_min_count() {
    let table = execute_read(
        "FOR n IN [1, 2, 2, 3] RETURN n \
         INTERSECT ALL FOR n IN [2, 2, 2, 4] RETURN n",
    );

    assert_eq!(
        column_values(&table, "n"),
        vec![Value::Int(2), Value::Int(2)]
    );
}

#[test]
fn except_returns_lhs_minus_rhs() {
    let table = execute_read("FOR n IN [1, 2, 3] RETURN n EXCEPT FOR n IN [2, 3, 4] RETURN n");

    assert_eq!(column_values(&table, "n"), vec![Value::Int(1)]);
}

#[test]
fn except_all_returns_saturating_sub_count() {
    let table = execute_read(
        "FOR n IN [1, 2, 2, 3] RETURN n \
         EXCEPT ALL FOR n IN [2, 4] RETURN n",
    );

    assert_eq!(
        column_values(&table, "n"),
        vec![Value::Int(1), Value::Int(2), Value::Int(3)]
    );
}

#[test]
fn otherwise_empty_lhs_evaluates_rhs() {
    let table = execute_read("FOR n IN [] RETURN n OTHERWISE RETURN 9 AS n");

    assert_eq!(column_values(&table, "n"), vec![Value::Int(9)]);
}

#[test]
fn otherwise_empty_lhs_yields_rhs_rows() {
    // ISO §14.2 SR v: OTHERWISE arms must be column name-equal. With matching
    // names, an empty LHS substitutes the RHS rows under the shared schema.
    // (Migrated from the prior `lhs`/`rhs` mismatch that relied on the lenient
    // LHS-relabel now rejected by `SetOpArmsNotCombinable`.)
    let table = execute_read("RETURN 1 AS v LIMIT 0 OTHERWISE RETURN 2 AS v");

    assert_eq!(column_values(&table, "v"), vec![Value::Int(2)]);
}

#[test]
fn otherwise_non_empty_lhs_does_not_evaluate_rhs() {
    let table = execute_read("RETURN 1 AS n OTHERWISE RETURN 1 / 0 AS n");

    assert_eq!(column_values(&table, "n"), vec![Value::Int(1)]);
}

#[test]
fn set_op_null_equality_via_runtime_eq_key() {
    let table = execute_read(
        "FOR n IN [NULL, NULL, 1] RETURN n \
         INTERSECT FOR n IN [NULL] RETURN n",
    );

    assert_eq!(column_values(&table, "n"), vec![Value::Null]);
}

#[test]
fn set_op_cross_type_numeric_equality() {
    let table = execute_read("RETURN 1 AS n INTERSECT RETURN 1.0 AS n");

    assert_eq!(column_values(&table, "n"), vec![Value::Int(1)]);
}

#[test]
fn set_op_string_equality() {
    let table = execute_manual_set_op(
        SetOp::Intersect,
        "RETURN 'same' AS s",
        string_table("s", vec![Value::String(db_string("same").unwrap())]),
        &ImplDefinedCaps::default(),
        None,
    )
    .expect("intersect executes");

    assert_eq!(
        column_values(&table, "s"),
        vec![Value::String(db_string("same").unwrap())]
    );
}

#[test]
fn set_op_schema_mismatch_emits_22g03() {
    let err = execute_manual_set_op(
        SetOp::Except,
        "RETURN 2 AS a, 3 AS b",
        int_table("n", [1]),
        &ImplDefinedCaps::default(),
        None,
    )
    .expect_err("schema mismatch errors");

    assert_eq!(err.gqlstatus().as_str(), "22G03");
    assert!(matches!(
        err,
        ExecutorError::DataException {
            subclass: DataExceptionSubclass::InvalidValueType,
            ..
        }
    ));
}

#[test]
fn set_op_cancellation_returns_5gql2() {
    let token = CancellationToken::new();
    token.cancel();
    let err = execute_manual_set_op(
        SetOp::ExceptAll,
        "RETURN 9999 AS n",
        int_table("n", 0..2048),
        &ImplDefinedCaps::default(),
        Some(&token),
    )
    .expect_err("cancelled set op errors");

    assert!(matches!(err, ExecutorError::Cancelled { .. }));
    assert_eq!(err.gqlstatus().as_str(), "5GQL2");
}

#[test]
fn set_op_key_cap_exceeded_returns_5gql1() {
    let caps =
        ImplDefinedCaps::default().with_set_op_key_cap(NonZeroUsize::new(1).expect("non-zero cap"));
    let err = execute_manual_set_op(
        SetOp::Intersect,
        "FOR n IN [1, 2] RETURN n",
        int_table("n", [1]),
        &caps,
        None,
    )
    .expect_err("key cap errors");

    assert_eq!(err.gqlstatus().as_str(), "5GQL1");
    assert!(matches!(
        err,
        ExecutorError::ProgramLimitExceeded {
            detail: "set-op key cap exceeded",
            ..
        }
    ));
}

#[test]
fn final_output_still_respects_session_row_cap() {
    let graph = SharedGraph::new(GraphId::new(8127));
    let mut session = Session::new(&graph).with_row_cap(1);
    let err = session
        .execute_source(
            "FOR n IN [1, 2] RETURN n EXCEPT RETURN 99 AS n",
            &EmptyProcedureRegistry,
        )
        .expect_err("outer result exceeds row cap");

    assert!(matches!(err, ExecutorError::RowCapExceeded { cap: 1, .. }));
    assert_eq!(err.gqlstatus().as_str(), "5GQL1");
}

#[test]
fn otherwise_empty_lhs_statement_output_rows() {
    let graph = SharedGraph::new(GraphId::new(9127));
    let mut session = Session::new(&graph);
    let output = session
        .execute_source(
            "FOR n IN [] RETURN n OTHERWISE RETURN 9 AS n",
            &EmptyProcedureRegistry,
        )
        .expect("otherwise statement executes");

    assert!(matches!(
        output,
        StatementOutput::Rows(table) if column_values(&table, "n") == vec![Value::Int(9)]
    ));
}
