//! Pipeline Let executor tests.

mod exec_common;

use exec_common::{
    ExecFixture, column_values, execute_pattern, execute_read, execute_read_result, planned,
};
use selene_core::Value;
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, ExecutorError, GqlType, PipelineOp,
    PipelineStatement, Statement, execute_pipeline, parse,
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
fn let_value_typed_binding_parse_carries_declared_type() {
    let statement = parse("LET VALUE x :: INTEGER = 1 RETURN x").expect("parse");
    let Statement::Query(pipeline) = statement else {
        panic!("expected query pipeline");
    };
    let PipelineStatement::Let(bindings) = &pipeline.statements[0] else {
        panic!("expected LET statement");
    };

    assert_eq!(bindings[0].alias.as_str(), "x");
    assert_eq!(bindings[0].declared_type, Some(GqlType::Integer));
}

#[test]
fn let_shorthand_alias_with_value_prefix_stays_identifier() {
    let table = execute_read("LET valuex = 7 RETURN valuex");

    assert_eq!(column_values(&table, "valuex"), vec![Value::Int(7)]);
}

#[test]
fn let_value_typed_binding_accepts_supported_spellings() {
    for source in [
        "LET VALUE x INTEGER = 42 RETURN x",
        "LET VALUE x TYPED INTEGER = 42 RETURN x",
        "LET VALUE x :: INTEGER = 42 RETURN x",
    ] {
        let table = execute_read(source);

        assert_eq!(column_values(&table, "x"), vec![Value::Int(42)], "{source}");
    }
}

#[test]
fn let_value_typed_binding_sets_output_schema_type() {
    let plan = planned("LET VALUE x :: INTEGER = 42 RETURN x");
    let PipelineOp::Let(items) = &plan.pipeline[0] else {
        panic!("expected LET op");
    };

    assert_eq!(
        items[0].ty,
        selene_gql::AnalyzedType::Resolved(GqlType::Integer)
    );
    assert_eq!(items[0].declared_type, Some(GqlType::Integer));
}

#[test]
fn let_value_typed_binding_rejects_runtime_mismatch() {
    let err =
        execute_read_result("LET VALUE x :: INTEGER = 'abc' RETURN x").expect_err("type mismatch");

    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name,
            ref expected,
            actual: "STRING",
            ..
        } if name.as_str() == "x" && expected.as_ref() == "INTEGER"
    ));
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
