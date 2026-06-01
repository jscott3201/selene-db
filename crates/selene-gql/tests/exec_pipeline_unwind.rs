//! Pipeline Unwind executor tests.

mod exec_common;

use exec_common::{ExecFixture, column_values, planned};
use selene_core::Value;
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, ExecutorError, Literal, PipelineOp, ValueExpr,
    execute_pipeline,
};

fn input() -> BindingTable {
    BindingTable::new(
        BindingTableSchema { columns: vec![] },
        vec![Binding::empty()],
    )
}

#[test]
fn unwind_emits_one_row_per_list_element() {
    let fixture = ExecFixture::build();
    let plan = planned("UNWIND [1, 2, 3] AS x RETURN x");
    let mut ctx = fixture.context_caps(&plan);

    let table = execute_pipeline(&plan.pipeline, input(), &mut ctx).expect("unwind executes");

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Int(1), Value::Int(2), Value::Int(3)]
    );
}

#[test]
fn unwind_of_empty_list_emits_zero_rows() {
    let fixture = ExecFixture::build();
    let plan = planned("UNWIND [] AS x RETURN x");
    let mut ctx = fixture.context_caps(&plan);

    let table = execute_pipeline(&plan.pipeline, input(), &mut ctx).expect("unwind executes");

    assert!(table.is_empty());
}

#[test]
fn unwind_of_null_emits_zero_rows() {
    let fixture = ExecFixture::build();
    let plan = planned("UNWIND NULL AS x RETURN x");
    let mut ctx = fixture.context_caps(&plan);

    let table = execute_pipeline(&plan.pipeline, input(), &mut ctx).expect("unwind executes");

    assert!(table.is_empty());
}

#[test]
fn unwind_of_non_list_returns_data_exception() {
    let fixture = ExecFixture::build();
    let plan = planned("UNWIND [1] AS x RETURN x");
    let mut ctx = fixture.context_caps(&plan);
    let mut op = plan.pipeline[0].clone();
    let PipelineOp::Unwind { source, span, .. } = &mut op else {
        panic!("expected unwind");
    };
    source.expr = ValueExpr::Literal(Literal::Integer(1, *span));

    let err = execute_pipeline(&[op], input(), &mut ctx).expect_err("unwind errors");

    assert!(matches!(err, ExecutorError::DataException { .. }));
}

#[test]
fn unwind_extends_schema_with_alias_column() {
    let fixture = ExecFixture::build();
    let plan = planned("UNWIND [1] AS x RETURN x");
    let mut ctx = fixture.context_caps(&plan);

    let table = execute_pipeline(&plan.pipeline[..1], input(), &mut ctx).expect("unwind executes");

    assert_eq!(
        table.schema().columns[0].name.clone().unwrap().as_str(),
        "x"
    );
}
