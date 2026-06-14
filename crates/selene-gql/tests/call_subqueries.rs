//! Inline CALL subquery execution tests.

mod exec_common;

use std::num::NonZeroUsize;

use exec_common::{ExecFixture, column_values};
use selene_core::Value;
use selene_gql::ast::format_read_statement;
use selene_gql::{
    AnalyzedType, EmptyProcedureRegistry, ExecutorError, GqlType, ProcedureMutability, Session,
    StatementOutput, analyze, parse, plan,
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
        vec![Value::String(exec_common::db_string("Bob")), Value::Null]
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
fn optional_call_subquery_with_empty_body_preserves_rows_with_null_yields() {
    let table = execute(
        "MATCH (a:Person)
         OPTIONAL CALL { MATCH (a)-[:KNOWS]->(:Nope) RETURN 1 AS n }
         YIELD n
         RETURN a.name AS name, n
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
    assert_eq!(
        value_values(&table, "n"),
        vec![Value::Null, Value::Null, Value::Null]
    );
}

#[test]
fn optional_call_subquery_without_yield_preserves_rows_for_empty_body() {
    let table = execute(
        "MATCH (a:Person)
         OPTIONAL CALL { MATCH (a)-[:KNOWS]->(:Nope) }
         RETURN a.name AS name
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
}

#[test]
fn optional_call_subquery_yield_schema_relaxes_non_null_columns() {
    let source =
        "OPTIONAL CALL { RETURN CAST(1 AS INTEGER NOT NULL) AS n LIMIT 0 } YIELD n RETURN n";
    let parsed = parse(source).expect("optional inline CALL parses");
    let analyzed = analyze(parsed, &EmptyProcedureRegistry, None).expect("query analyzes");
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("query plans");

    assert_eq!(
        plan.output_schema.columns[0].ty,
        AnalyzedType::Resolved(GqlType::Integer)
    );
}

#[test]
fn optional_call_subquery_null_yield_does_not_satisfy_not_null_type_check() {
    let table = execute(
        "OPTIONAL CALL { RETURN CAST(1 AS INTEGER NOT NULL) AS n LIMIT 0 }
         YIELD n
         RETURN n IS TYPED INTEGER NOT NULL AS ok",
    );

    assert_eq!(value_values(&table, "ok"), vec![Value::Bool(false)]);
}

#[test]
fn optional_call_subquery_formats_and_reparses() {
    let parsed = parse("OPTIONAL CALL { RETURN 1 AS one LIMIT 1 } YIELD one")
        .expect("optional inline CALL parses");
    let formatted = format_read_statement(&parsed).expect("optional inline CALL formats");

    assert_eq!(
        formatted,
        "OPTIONAL CALL { RETURN 1 AS one\nLIMIT 1 } YIELD one"
    );
    parse(&formatted).expect("formatted optional inline CALL reparses");
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

// GP03 (ISO/IEC 39075:2024 §15.2): explicit variable-scope CALL subqueries.
// The body sees ONLY the named imports; an empty `()` is fully isolated.

#[test]
fn gp03_imports_named_outer_binding() {
    // `a` is imported, so `a.name` resolves inside the body — one row per outer
    // Person, carrying that person's name.
    let table = execute(
        "MATCH (a:Person) CALL (a) { RETURN a.name AS n LIMIT 1 } YIELD n RETURN n ORDER BY n",
    );
    assert_eq!(
        string_values(&table, "n"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
}

#[test]
fn gp03_empty_scope_executes_isolated_body() {
    // `CALL () { ... }` imports nothing; a self-contained body still runs once
    // per outer row.
    let table =
        execute("MATCH (a:Person) CALL () { RETURN 1 AS n LIMIT 1 } YIELD n RETURN n ORDER BY n");
    assert_eq!(int_values(&table, "n"), vec![1, 1, 1]);
}

#[test]
fn gp03_unimported_outer_binding_is_out_of_scope() {
    // `b` is an outer binding but is NOT in the import list, so it is invisible
    // inside the subquery — an undefined reference (42N03). This is the core
    // GP03 restriction.
    assert_status(
        "MATCH (a:Person) MATCH (b:Person) CALL (a) { RETURN b.name AS n LIMIT 1 } YIELD n RETURN n",
        "42N03",
    );
}

#[test]
fn gp03_empty_scope_rejects_outer_reference() {
    assert_status(
        "MATCH (a:Person) CALL () { RETURN a.name AS n LIMIT 1 } YIELD n RETURN n",
        "42N03",
    );
}

#[test]
fn gp03_unknown_import_name_is_undefined() {
    assert_status(
        "MATCH (a:Person) CALL (nonesuch) { RETURN 1 AS n LIMIT 1 } YIELD n RETURN n",
        "42N03",
    );
}

#[test]
fn gp03_duplicate_import_is_rejected() {
    assert_status(
        "MATCH (a:Person) CALL (a, a) { RETURN 1 AS n LIMIT 1 } YIELD n RETURN n",
        "42N10",
    );
}

#[test]
fn gp03_pattern_reuse_of_import_executes_cleanly() {
    // Reusing the imported `a` in a labeled pattern (`a:Sensor`) executes without
    // error: each outer Person `a` is constrained by the inner `:Sensor`, which
    // matches nothing (single-label fixture), so the `CALL{}` semi-join drops
    // every row → empty result. The point is that labeled reuse of an import is
    // handled (the read-only-import path), not a panic or analysis error.
    // (The no-leak property — the inner label must not corrupt the OUTER scan's
    // label predicate — is asserted at the plan level in plan_read_pipeline.rs,
    // because the conflicting label masks it in execution results.)
    let table = execute(
        "MATCH (a:Person) CALL (a) { MATCH (a:Sensor) RETURN 1 AS n LIMIT 1 } YIELD n RETURN n",
    );
    assert!(table.is_empty());
}

#[test]
fn call_subquery_rejects_write_inside_body() {
    let registry = MockProcedureRegistry::new().with_procedure_mutability(
        vec![exec_common::db_string("mutate")],
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
