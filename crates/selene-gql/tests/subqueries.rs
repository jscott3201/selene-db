//! Expression-subquery execution tests.

mod exec_common;

use exec_common::{ExecFixture, column_values, execute_plan as execute_planned_pipeline, planned};
use selene_core::Value;
use selene_gql::{EmptyProcedureRegistry, ExecutorError, Session, StatementOutput};

fn execute(source: &str) -> selene_gql::BindingTable {
    execute_result(source).expect("query executes")
}

fn execute_result(source: &str) -> Result<selene_gql::BindingTable, ExecutorError> {
    let fixture = ExecFixture::build();
    let mut session = Session::new(&fixture.graph);
    match session.execute_source(source, &EmptyProcedureRegistry)? {
        StatementOutput::Rows(table) => Ok(table),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn bool_values(table: &selene_gql::BindingTable, name: &str) -> Vec<bool> {
    column_values(table, name)
        .into_iter()
        .map(|value| match value {
            Value::Bool(value) => value,
            other => panic!("expected bool, got {other:?}"),
        })
        .collect()
}

fn int_values(table: &selene_gql::BindingTable, name: &str) -> Vec<i64> {
    column_values(table, name)
        .into_iter()
        .map(|value| match value {
            Value::Int(value) => value,
            other => panic!("expected int, got {other:?}"),
        })
        .collect()
}

fn string_values(table: &selene_gql::BindingTable, name: &str) -> Vec<String> {
    column_values(table, name)
        .into_iter()
        .map(|value| match value {
            Value::String(value) => value.as_str().to_owned(),
            Value::ExternalString(value) => value.as_ref().to_owned(),
            other => panic!("expected string, got {other:?}"),
        })
        .collect()
}

fn value_values(table: &selene_gql::BindingTable, name: &str) -> Vec<Value> {
    column_values(table, name)
}

fn assert_status(source: &str, status: &str) {
    let err = execute_result(source).expect_err("query rejects");
    assert_eq!(err.gqlstatus().as_str(), status);
}

#[test]
fn exists_empty_returns_false() {
    let table = execute("RETURN EXISTS { MATCH (:Nope) } AS e");

    assert_eq!(bool_values(&table, "e"), vec![false]);
}

#[test]
fn exists_non_empty_returns_true() {
    let table = execute("RETURN EXISTS { MATCH (:Person) } AS e");

    assert_eq!(bool_values(&table, "e"), vec![true]);
}

#[test]
fn not_exists_returns_negation() {
    let table = execute("RETURN NOT EXISTS { MATCH (:Nope) } AS e");

    assert_eq!(bool_values(&table, "e"), vec![true]);
}

#[test]
fn exists_correlated_with_outer_binding() {
    let table = execute(
        "MATCH (a:Person)
         RETURN a.name AS name, EXISTS { MATCH (a)-[:KNOWS]->(:Sensor) } AS has_sensor
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
    assert_eq!(bool_values(&table, "has_sensor"), [false, true, false]);
}

#[test]
fn exists_nested_two_levels() {
    let table = execute(
        "MATCH (a:Person)
         RETURN a.name AS name,
                EXISTS {
                  MATCH (a)-[:KNOWS]->(b)
                  WHERE EXISTS { MATCH (b)-[:KNOWS]->(:Sensor) }
                } AS has_two_hop_sensor
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
    assert_eq!(
        bool_values(&table, "has_two_hop_sensor"),
        [true, false, false]
    );
}

#[test]
fn exists_correlates_when_outer_binding_only_appears_in_where() {
    let table = execute(
        "MATCH (a:Person)
         RETURN a.name AS name,
                EXISTS {
                  MATCH (b:Person)
                  WHERE b = a
                } AS has_same_node
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
    assert_eq!(bool_values(&table, "has_same_node"), [true, true, true]);
}

#[test]
fn null_correlated_seed_returns_empty_subquery_result() {
    let table = execute(
        "MATCH (a:Person)
         OPTIONAL MATCH (a)-[:KNOWS]->(m:Sensor)
         RETURN a.name AS name, EXISTS { MATCH (m)-[:KNOWS]->() } AS has_sensor_outgoing
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
    assert_eq!(
        bool_values(&table, "has_sensor_outgoing"),
        [false, false, false]
    );
}

#[test]
fn public_pipeline_execution_uses_attached_plan_subquery_metadata() {
    let fixture = ExecFixture::build();
    let plan = planned(
        "MATCH (a:Person)
         RETURN a.name AS name, EXISTS { MATCH (a)-[:KNOWS]->() } AS has_outgoing
         ORDER BY name",
    );

    let table = execute_planned_pipeline(&fixture, &plan).expect("pipeline executes");

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
    assert_eq!(bool_values(&table, "has_outgoing"), [true, true, false]);
}

#[test]
fn count_subquery_empty_returns_zero() {
    let table = execute("RETURN COUNT { MATCH (:Nope) } AS c");

    assert_eq!(int_values(&table, "c"), vec![0]);
}

#[test]
fn count_subquery_returns_row_count() {
    let table = execute("RETURN COUNT { MATCH (:Person) } AS c");

    assert_eq!(int_values(&table, "c"), vec![3]);
}

#[test]
fn count_subquery_correlates_with_outer_binding() {
    let table = execute(
        "MATCH (a:Person)
         RETURN a.name AS name, COUNT { MATCH (a)-[:KNOWS]->() } AS outgoing
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
    assert_eq!(int_values(&table, "outgoing"), [1, 1, 0]);
}

#[test]
fn value_subquery_returns_uncorrelated_scalar() {
    let table = execute("RETURN VALUE { RETURN 7 LIMIT 1 } AS v");

    assert_eq!(int_values(&table, "v"), vec![7]);
}

#[test]
fn value_subquery_returns_null_for_empty_result() {
    let table = execute("RETURN VALUE { MATCH (:Nope) RETURN 1 LIMIT 1 } AS v");

    assert_eq!(value_values(&table, "v"), vec![Value::Null]);
}

#[test]
fn value_subquery_allows_direct_aggregate_without_limit() {
    let table = execute("RETURN VALUE { MATCH (n:Person) RETURN count(n) } AS c");

    assert_eq!(int_values(&table, "c"), vec![3]);
}

#[test]
fn value_subquery_allows_non_aggregate_with_limit_one() {
    let table = execute("RETURN VALUE { MATCH (n:Person) RETURN n.name LIMIT 1 } AS name");

    assert_eq!(string_values(&table, "name"), vec!["Alice".to_owned()]);
}

#[test]
fn value_subquery_correlates_with_outer_binding() {
    let table = execute(
        "MATCH (a:Person)
         RETURN a.name AS name,
                VALUE { MATCH (a)-[:KNOWS]->(b) RETURN b.name LIMIT 1 } AS first_known
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
    assert_eq!(
        value_values(&table, "first_known"),
        vec![
            Value::String(exec_common::istr("Bob")),
            Value::Null,
            Value::Null
        ]
    );
}

#[test]
fn value_subquery_rejects_multi_item_return_as_syntax_error() {
    assert_status(
        "RETURN VALUE { RETURN 1 AS a, 2 AS b LIMIT 1 } AS v",
        "42001",
    );
}

#[test]
fn value_subquery_rejects_non_aggregate_without_limit_one() {
    assert_status("RETURN VALUE { RETURN 1 } AS v", "42001");
}

#[test]
fn value_subquery_rejects_missing_return() {
    assert_status("RETURN VALUE { MATCH (:Person) } AS v", "42001");
}

#[test]
fn value_subquery_depth_guard_limits_nested_subqueries() {
    let mut source = String::from("RETURN ");
    for _ in 0..260 {
        source.push_str("VALUE { RETURN ");
    }
    source.push('1');
    for _ in 0..260 {
        source.push_str(" LIMIT 1 }");
    }
    source.push_str(" AS v");

    assert_status(&source, "5GQL1");
}

#[test]
fn expression_subquery_rejects_multiple_match_clauses() {
    let err = execute_result("RETURN EXISTS { MATCH (a) MATCH (b) } AS e")
        .expect_err("multi-MATCH subquery is syntax error");

    assert!(matches!(err, ExecutorError::Parse { .. }));
}
