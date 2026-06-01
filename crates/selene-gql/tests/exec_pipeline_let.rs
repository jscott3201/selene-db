//! Pipeline Let executor tests.

mod exec_common;

use exec_common::{ExecFixture, column_values, execute_pattern, planned};
use selene_core::Value;
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, ExecutorError, PipelineOp, execute_pipeline,
};

#[test]
fn let_extends_schema_preserving_existing_columns() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (n:Person) LET doubled = n.age + n.age RETURN n, doubled");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let mut ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);
    let let_op = plan
        .pipeline
        .iter()
        .find(|op| matches!(op, PipelineOp::Let(_)))
        .expect("let op")
        .clone();

    let extended = execute_pipeline(&[let_op], table, &mut ctx).expect("let executes");

    let names = extended
        .schema()
        .columns
        .iter()
        .filter_map(|column| column.name.clone().map(|name| name.as_str().to_owned()))
        .collect::<Vec<_>>();
    assert!(names.contains(&"n".to_owned()));
    assert!(names.contains(&"doubled".to_owned()));
}

#[test]
fn let_evaluates_expression_per_row_against_existing_bindings() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (n:Person) LET doubled = n.age + n.age RETURN n, doubled");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let mut ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);
    let let_op = plan
        .pipeline
        .iter()
        .find(|op| matches!(op, PipelineOp::Let(_)))
        .expect("let op")
        .clone();

    let extended = execute_pipeline(&[let_op], table, &mut ctx).expect("let executes");

    assert_eq!(
        column_values(&extended, "doubled"),
        vec![Value::Int(60), Value::Int(84), Value::Int(110)]
    );
}

#[test]
fn let_evaluates_items_in_order_with_progressive_visibility() {
    let fixture = ExecFixture::build();
    let plan = planned("LET a = 1, b = a + 1 RETURN a, b");
    let mut ctx = fixture.context_caps(&plan);
    let input = BindingTable::new(
        BindingTableSchema { columns: vec![] },
        vec![Binding::empty()],
    );

    let table = execute_pipeline(&plan.pipeline, input, &mut ctx).expect("pipeline executes");

    assert_eq!(column_values(&table, "a"), vec![Value::Int(1)]);
    assert_eq!(column_values(&table, "b"), vec![Value::Int(2)]);
}

#[test]
fn let_propagates_evaluator_errors() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (n:Person) LET bad = 1 / 0 RETURN bad");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let mut ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);
    let let_op = plan
        .pipeline
        .iter()
        .find(|op| matches!(op, PipelineOp::Let(_)))
        .expect("let op")
        .clone();

    let err = execute_pipeline(&[let_op], table, &mut ctx).expect_err("let errors");

    assert!(matches!(err, ExecutorError::DataException { .. }));
}
