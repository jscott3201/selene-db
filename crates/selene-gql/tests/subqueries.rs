//! Expression-subquery execution tests.

mod exec_common;

use exec_common::{ExecFixture, column_values, execute_plan as execute_planned_pipeline, planned};
use selene_core::Value;
use selene_gql::{
    EmptyProcedureRegistry, ExecutorError, Session, StatementOutput,
    ast::{format_read_statement, structurally_eq},
    parse,
};

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
fn exists_accepts_direct_graph_pattern_body() {
    let table = execute("RETURN EXISTS { (:Person) } AS braced, EXISTS ((:Person)) AS paren");

    assert_eq!(bool_values(&table, "braced"), vec![true]);
    assert_eq!(bool_values(&table, "paren"), vec![true]);
}

#[test]
fn exists_accepts_parenthesized_match_body() {
    let table = execute("RETURN EXISTS (MATCH (:Person)) AS e");

    assert_eq!(bool_values(&table, "e"), vec![true]);
}

#[test]
fn not_exists_returns_negation() {
    let table = execute("RETURN NOT EXISTS { MATCH (:Nope) } AS e");

    assert_eq!(bool_values(&table, "e"), vec![true]);
}

#[test]
fn not_exists_accepts_parenthesized_direct_graph_pattern_body() {
    let table = execute("RETURN NOT EXISTS ((:Nope)) AS e");

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
fn direct_exists_graph_pattern_correlates_with_outer_binding() {
    let table = execute(
        "MATCH (a:Person)
         RETURN a.name AS name, EXISTS { (a)-[:KNOWS]->(:Sensor) } AS has_sensor
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
    assert_eq!(bool_values(&table, "has_sensor"), [false, true, false]);
}

#[test]
fn direct_exists_graph_pattern_formats_as_canonical_match_body() {
    let parsed = parse("RETURN EXISTS { (:Person) } AS e").expect("direct EXISTS parses");
    let formatted = format_read_statement(&parsed).expect("read statement formats");
    assert_eq!(formatted, "RETURN EXISTS { MATCH (:Person) } AS e");
    let reparsed = parse(&formatted).expect("formatted EXISTS parses");
    assert!(structurally_eq(&parsed, &reparsed));
}

/// GQLRT-05 regression: two distinct correlated subqueries in one statement must
/// keep independent, correctly-correlated results across every outer row. The
/// per-statement target-schema memo is keyed by expression id and populated on
/// the first outer row, then reused — so this exercises both the cross-subquery
/// keying (the two `EXISTS` get different schemas/labels) and the cache-hit path
/// (rows 2..N reuse the row-1 schema while still correlating per row).
#[test]
fn distinct_correlated_subqueries_stay_independent_under_schema_memo() {
    let table = execute(
        "MATCH (a:Person)
         RETURN a.name AS name,
                EXISTS { MATCH (a)-[:KNOWS]->(:Person) } AS knows_person,
                EXISTS { MATCH (a)-[:KNOWS]->(:Sensor) } AS knows_sensor
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
    // Alice KNOWS Bob (a Person); Bob KNOWS the Sensor; Cara has no out-edges.
    // The two columns must diverge — a memo that cross-wired the schemas or
    // failed to re-correlate per row would collapse them.
    assert_eq!(bool_values(&table, "knows_person"), [true, false, false]);
    assert_eq!(bool_values(&table, "knows_sensor"), [false, true, false]);
}

/// GQLRT-05 regression: a subquery nested inside another subquery crosses the
/// `EvalCtx::with_plan` boundary (a distinct plan registry) yet shares the one
/// per-statement schema memo. Because expression ids are allocated once per
/// statement (a single `ExprTypeTable`) and every plan clones that one
/// `ExprIdLookup`, the outer and inner `EXISTS` get distinct ids that cannot
/// collide in the cache. This asserts the nested correlation still resolves
/// per row through the memo.
#[test]
fn nested_correlated_subquery_through_memo_is_correct() {
    let table = execute(
        "MATCH (a:Person)
         WHERE EXISTS {
             MATCH (a)-[:KNOWS]->(b)
             WHERE EXISTS { MATCH (b)-[:KNOWS]->(:Sensor) }
         }
         RETURN a.name AS name
         ORDER BY name",
    );

    // Alice KNOWS Bob, and Bob KNOWS the Sensor -> Alice qualifies.
    // Bob KNOWS the Sensor, but the Sensor has no outgoing KNOWS -> Bob fails.
    // Cara has no out-edges -> fails.
    assert_eq!(string_values(&table, "name"), vec!["Alice".to_owned()]);
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
fn value_subquery_preserves_null_outer_binding() {
    let table = execute(
        "MATCH (a:Person)
         OPTIONAL MATCH (a)-[:KNOWS]->(m:Sensor)
         RETURN a.name AS name, VALUE { RETURN m IS NULL LIMIT 1 } AS missing_sensor
         ORDER BY name",
    );

    assert_eq!(
        string_values(&table, "name"),
        vec!["Alice".to_owned(), "Bob".to_owned(), "Cara".to_owned()]
    );
    assert_eq!(bool_values(&table, "missing_sensor"), [true, false, true]);
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
            Value::String(exec_common::db_string("Bob")),
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
fn value_subquery_shape_uses_final_return_clause() {
    assert_status(
        "RETURN VALUE { RETURN 1 AS a LIMIT 1 RETURN 1 AS a, 2 AS b LIMIT 1 } AS v",
        "42001",
    );
}

#[test]
fn value_subquery_rejects_non_aggregate_without_limit_one() {
    assert_status("RETURN VALUE { RETURN 1 } AS v", "42001");
}

#[test]
fn value_subquery_limit_one_must_bound_final_result() {
    assert_status(
        "RETURN VALUE { RETURN 1 LIMIT 1 FOR n IN [1, 2] RETURN n } AS v",
        "42001",
    );
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
fn exists_subquery_depth_guard_visits_match_body_expressions() {
    let mut source = String::from("RETURN EXISTS { MATCH (n) WHERE ");
    for _ in 0..260 {
        source.push_str("VALUE { RETURN ");
    }
    source.push('1');
    for _ in 0..260 {
        source.push_str(" LIMIT 1 }");
    }
    source.push_str(" = 1 } AS e");

    assert_status(&source, "5GQL1");
}

#[test]
fn expression_subquery_rejects_multiple_match_clauses() {
    let err = execute_result("RETURN EXISTS { MATCH (a) MATCH (b) } AS e")
        .expect_err("multi-MATCH subquery is syntax error");

    assert!(matches!(err, ExecutorError::Parse { .. }));
}
