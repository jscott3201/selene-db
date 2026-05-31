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

fn two_column_table(rows: Vec<[Value; 2]>) -> BindingTable {
    BindingTable::new(
        BindingTableSchema {
            columns: vec![
                BindingTableColumn {
                    name: Some(intern("a").expect("interns")),
                    hidden: None,
                    ty: AnalyzedType::Resolved(GqlType::Integer),
                },
                BindingTableColumn {
                    name: Some(intern("b").expect("interns")),
                    hidden: None,
                    ty: AnalyzedType::Resolved(GqlType::Integer),
                },
            ],
        },
        rows.into_iter().map(Binding::new).collect(),
    )
}

fn distinct_table(table: BindingTable) -> BindingTable {
    let caps = ImplDefinedCaps::default();
    let mut ctx = exec_common::empty_graph_context(&caps);
    execute_pipeline(&[PipelineOp::Distinct], table, &mut ctx).expect("distinct executes")
}

#[test]
fn distinct_collapses_multiple_null_rows() {
    // GQLRT-33: DISTINCT keys rows through the runtime equality key, where
    // (Null, Null) => true. Three NULL rows must collapse to one. Previously
    // this was covered only transitively by the RuntimeEqKey unit test, never
    // end-to-end through the Distinct pipeline op.
    let table = distinct(vec![Value::Null, Value::Null, Value::Null]);

    assert_eq!(column_values(&table, "v"), vec![Value::Null]);
}

#[test]
fn distinct_collapses_null_in_one_key_column_of_two() {
    // A two-column case where the FIRST column is NULL in several rows: rows
    // are distinct only by their second column, so NULL-keyed rows must not all
    // collapse — only rows equal in BOTH columns dedup.
    let table = distinct_table(two_column_table(vec![
        [Value::Null, Value::Int(1)],
        [Value::Null, Value::Int(1)], // exact duplicate of row 0 -> collapses
        [Value::Null, Value::Int(2)], // distinct in column b -> kept
        [Value::Int(9), Value::Int(2)],
    ]));

    assert_eq!(
        column_values(&table, "a"),
        vec![Value::Null, Value::Null, Value::Int(9)]
    );
    assert_eq!(
        column_values(&table, "b"),
        vec![Value::Int(1), Value::Int(2), Value::Int(2)]
    );
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
fn distinct_dedups_lossless_int_and_float_equivalents() {
    let table = distinct(vec![Value::Int(1), Value::Float(1.0)]);

    assert_eq!(column_values(&table, "v"), vec![Value::Int(1)]);
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
        vec![Value::Int(LARGE_COUNTER_A), Value::Int(LARGE_COUNTER_B)]
    );
}
