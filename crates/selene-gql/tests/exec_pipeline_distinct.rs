//! Pipeline Distinct executor tests.

mod exec_common;

use exec_common::{LARGE_COUNTER_A, LARGE_COUNTER_B, column_values};
use selene_core::{Value, intern};
use selene_gql::{
    AnalyzedType, Binding, BindingTable, BindingTableColumn, BindingTableSchema, GqlType,
    ImplDefinedCaps, PipelineOp, execute_pipeline,
};

fn table(values: Vec<Value>) -> BindingTable {
    BindingTable::new(
        BindingTableSchema {
            columns: vec![BindingTableColumn {
                name: Some(intern("v").expect("interns")),
                hidden: None,
                ty: AnalyzedType::Resolved(GqlType::Integer),
            }],
        },
        values
            .into_iter()
            .map(|value| Binding::new([value]))
            .collect(),
    )
}

fn distinct(values: Vec<Value>) -> BindingTable {
    let caps = ImplDefinedCaps::default();
    let mut ctx = exec_common::empty_graph_context(&caps);
    execute_pipeline(&[PipelineOp::Distinct], table(values), &mut ctx).expect("distinct executes")
}

#[test]
fn distinct_dedups_identical_rows() {
    let table = distinct(vec![Value::Int(1), Value::Int(1), Value::Int(2)]);

    assert_eq!(
        column_values(&table, "v"),
        vec![Value::Int(1), Value::Int(2)]
    );
}

#[test]
fn distinct_preserves_first_occurrence_order() {
    let table = distinct(vec![Value::Int(2), Value::Int(1), Value::Int(2)]);

    assert_eq!(
        column_values(&table, "v"),
        vec![Value::Int(2), Value::Int(1)]
    );
}

#[test]
fn distinct_treats_int_and_float_as_distinct() {
    let table = distinct(vec![Value::Int(1), Value::Float(1.0)]);

    assert_eq!(
        column_values(&table, "v"),
        vec![Value::Int(1), Value::Float(1.0)]
    );
}

#[test]
fn distinct_uses_lossless_numeric_equality() {
    let table = distinct(vec![
        Value::Int(LARGE_COUNTER_A),
        Value::Int(LARGE_COUNTER_B),
        Value::Float(LARGE_COUNTER_A as f64),
    ]);

    assert_eq!(
        column_values(&table, "v"),
        vec![
            Value::Int(LARGE_COUNTER_A),
            Value::Int(LARGE_COUNTER_B),
            Value::Float(LARGE_COUNTER_A as f64),
        ]
    );
}
