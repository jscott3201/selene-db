//! Inline CALL subquery execution tests.

mod exec_common;

use std::num::NonZeroUsize;

use exec_common::{ExecFixture, column_values};
use selene_core::Value;
use selene_gql::{
    EmptyProcedureRegistry, ExecutorError, ProcedureMutability, Session, StatementOutput,
};
use selene_testing::MockProcedureRegistry;

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

fn assert_status_with_registry(source: &str, registry: &MockProcedureRegistry, status: &str) {
    let fixture = ExecFixture::build();
    let mut session = Session::new(&fixture.graph);
    let err = session
        .execute_source(source, registry)
        .expect_err("query rejects");
    assert_eq!(err.gqlstatus().as_str(), status);
}

#[test]
fn call_subquery_runs_non_correlated_body_per_input_row() {
    let table = execute(
        "MATCH (a:Person)
         CALL { RETURN 1 AS one LIMIT 1 } YIELD one
         RETURN a.name AS name, one
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
    assert_eq!(int_values(&table, "one"), vec![1, 1, 1]);
}

#[test]
fn call_subquery_correlates_with_outer_binding() {
    let table = execute(
        "MATCH (a:Person)
         CALL { MATCH (a)-[:KNOWS]->(b) RETURN b.name AS known LIMIT 1 } YIELD known
         RETURN a.name AS name, known
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned()]
    );
    assert_eq!(
        value_values(&table, "known"),
        vec![Value::String(exec_common::istr("Bob")), Value::Null]
    );
}

#[test]
fn call_subquery_preserves_null_outer_binding() {
    let table = execute(
        "MATCH (a:Person)
         OPTIONAL MATCH (a)-[:KNOWS]->(m:Sensor)
         CALL { RETURN m IS NULL AS missing_sensor LIMIT 1 } YIELD missing_sensor
         RETURN a.name AS name, missing_sensor
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
    assert_eq!(
        value_values(&table, "missing_sensor"),
        vec![Value::Bool(true), Value::Bool(false), Value::Bool(true)]
    );
}

#[test]
fn call_subquery_without_yield_drops_outer_rows_when_inner_is_empty() {
    let table = execute(
        "MATCH (a:Person)
         CALL { MATCH (a)-[:KNOWS]->(:Nope) }
         RETURN a.name AS name
         ORDER BY name",
    );

    assert!(table.is_empty());
}

#[test]
fn call_subquery_without_yield_preserves_rows_for_unit_result() {
    let table = execute(
        "MATCH (a:Person)
         CALL { RETURN 1 LIMIT 1 }
         RETURN a.name AS name
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
}

#[test]
fn call_subquery_yield_as_renames_output_column() {
    let table = execute(
        "LET seed = 0
         CALL { RETURN 7 AS inner_value LIMIT 1 } YIELD inner_value AS renamed
         RETURN renamed",
    );

    assert_eq!(int_values(&table, "renamed"), vec![7]);
}

#[test]
fn call_subquery_runs_after_with_projection() {
    let table = execute(
        "MATCH (a:Person)
         WITH a.name AS name
         CALL { RETURN 1 AS one LIMIT 1 } YIELD one
         RETURN name, one
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
    assert_eq!(int_values(&table, "one"), vec![1, 1, 1]);
}

#[test]
fn call_subquery_rejects_in_transactions() {
    assert_status("CALL { RETURN 1 LIMIT 1 } IN TRANSACTIONS", "42N01");
}

#[test]
fn call_subquery_rejects_explicit_variable_scope() {
    assert_status(
        "MATCH (a:Person) CALL (a) { RETURN 1 AS one LIMIT 1 } YIELD one RETURN one",
        "42N01",
    );
}

#[test]
fn call_subquery_rejects_write_inside_body() {
    let registry = MockProcedureRegistry::new().with_procedure_mutability(
        vec![exec_common::istr("mutate")],
        Vec::new(),
        Vec::new(),
        ProcedureMutability::SchemaWrite,
    );

    assert_status_with_registry(
        "LET seed = 0 CALL { CALL mutate() RETURN 1 AS n } YIELD n RETURN n",
        &registry,
        "42N01",
    );
}

#[test]
fn plan_cache_hits_for_cacheable_call_subquery() {
    let fixture = ExecFixture::build();
    let mut session =
        Session::new(&fixture.graph).with_plan_cache(NonZeroUsize::new(8).expect("nonzero"));
    let source = "LET seed = 0 CALL { RETURN 1 AS one LIMIT 1 } YIELD one RETURN one";

    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("first execution");
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("second execution");

    let stats = session.plan_cache_stats().expect("cache enabled");
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hits, 1);
}
