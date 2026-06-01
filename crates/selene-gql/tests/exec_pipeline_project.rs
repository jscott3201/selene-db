//! Pipeline Project executor tests.

mod exec_common;

use exec_common::{ExecFixture, execute_pattern, planned};
use selene_core::Value;
use selene_gql::{ExecutorError, execute_pipeline};

fn execute_no_pattern(source: &str) -> selene_gql::BindingTable {
    let plan = planned(source);
    let fixture = ExecFixture::build();
    let mut ctx = fixture.context_caps(&plan);
    let input = selene_gql::BindingTable::new(
        selene_gql::BindingTableSchema { columns: vec![] },
        vec![selene_gql::Binding::empty()],
    );
    execute_pipeline(&plan.pipeline, input, &mut ctx).expect("pipeline executes")
}

#[test]
fn project_replaces_schema_with_named_aliases() {
    let table = execute_no_pattern("RETURN 1 AS one, 2 AS two");

    let names = table
        .schema()
        .columns
        .iter()
        .map(|column| {
            column
                .name
                .clone()
                .expect("named column")
                .as_str()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(names, ["one", "two"]);
}

#[test]
fn project_evaluates_expression_per_row() {
    let table = execute_no_pattern("RETURN 2 + 3 AS five");

    assert_eq!(
        exec_common::column_values(&table, "five"),
        vec![Value::Int(5)]
    );
}

#[test]
fn project_handles_anonymous_alias() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (n:Person) RETURN n.name");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let mut ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);

    let projected = execute_pipeline(&plan.pipeline, table, &mut ctx).expect("project executes");

    assert_eq!(projected.schema().columns[0].name, None);
    assert_eq!(projected.row_count(), 3);
}

#[test]
fn project_propagates_evaluator_errors() {
    let plan = planned("RETURN 1 / 0 AS bad");
    let fixture = ExecFixture::build();
    let mut ctx = fixture.context_caps(&plan);
    let input = selene_gql::BindingTable::new(
        selene_gql::BindingTableSchema { columns: vec![] },
        vec![selene_gql::Binding::empty()],
    );

    let err = execute_pipeline(&plan.pipeline, input, &mut ctx).expect_err("project errors");

    assert!(matches!(err, ExecutorError::DataException { .. }));
}
