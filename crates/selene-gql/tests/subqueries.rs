//! Expression-subquery execution tests.

mod exec_common;

use exec_common::{ExecFixture, column_values};
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
fn expression_subquery_rejects_multiple_match_clauses() {
    let err = execute_result("RETURN EXISTS { MATCH (a) MATCH (b) } AS e")
        .expect_err("multi-MATCH subquery is syntax error");

    assert!(matches!(err, ExecutorError::Parse { .. }));
}
