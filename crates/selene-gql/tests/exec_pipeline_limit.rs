//! Pipeline Limit executor tests.

use selene_core::{Value, intern};
use selene_gql::{
    AnalyzedType, Binding, BindingTable, BindingTableColumn, BindingTableSchema, ExecutorError,
    GqlType, ImplDefinedCaps, LimitAmount, PipelineOp, TxContext, execute_pipeline,
};

fn table() -> BindingTable {
    BindingTable::new(
        BindingTableSchema {
            columns: vec![BindingTableColumn {
                name: Some(intern("n").expect("interns")),
                hidden: None,
                ty: AnalyzedType::Resolved(GqlType::Integer),
            }],
        },
        (0..5)
            .map(|value| Binding::new([Value::Int(value)]))
            .collect(),
    )
}

fn ctx<'a>(caps: &'a ImplDefinedCaps) -> TxContext<'a, 'a> {
    exec_common::empty_graph_context(caps)
}

mod exec_common;

fn execute(offset: u64, count: u64) -> BindingTable {
    let caps = ImplDefinedCaps::default();
    let mut ctx = ctx(&caps);
    execute_pipeline(
        &[PipelineOp::Limit {
            offset: LimitAmount::Literal(offset),
            count: LimitAmount::Literal(count),
        }],
        table(),
        &mut ctx,
    )
    .expect("limit executes")
}

#[test]
fn limit_with_offset_zero_count_n_returns_first_n() {
    assert_eq!(
        exec_common::column_values(&execute(0, 2), "n"),
        vec![Value::Int(0), Value::Int(1)]
    );
}

#[test]
fn limit_with_offset_n_count_m_returns_n_to_n_plus_m() {
    assert_eq!(
        exec_common::column_values(&execute(2, 2), "n"),
        vec![Value::Int(2), Value::Int(3)]
    );
}

#[test]
fn limit_with_offset_beyond_input_returns_empty() {
    assert!(execute(10, 2).is_empty());
}

#[test]
fn limit_with_count_beyond_input_saturates() {
    assert_eq!(
        exec_common::column_values(&execute(3, 100), "n"),
        vec![Value::Int(3), Value::Int(4)]
    );
}

#[test]
fn limit_parameter_is_implementation_defined_until_parameter_binding_lands() {
    let caps = ImplDefinedCaps::default();
    let mut ctx = ctx(&caps);

    let err = execute_pipeline(
        &[PipelineOp::Limit {
            offset: LimitAmount::Parameter(intern("rows").expect("interns")),
            count: LimitAmount::Literal(1),
        }],
        table(),
        &mut ctx,
    )
    .expect_err("parameter limit errors");

    assert!(matches!(err, ExecutorError::ImplementationDefined { .. }));
}
