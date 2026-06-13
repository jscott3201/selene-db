//! End-to-end read execution tests for pattern plus pipeline.

mod exec_common;

use exec_common::{ExecFixture, column_values, execute_pattern, planned};
use selene_core::Value;
use selene_gql::{Binding, BindingTable, BindingTableSchema, execute_pipeline};

fn execute_read(source: &str) -> BindingTable {
    let fixture = ExecFixture::build();
    let plan = planned(source);
    let mut ctx = fixture.context_caps(&plan);
    let input = if let Some(pattern) = &plan.pattern_plan {
        execute_pattern(pattern, &ctx)
    } else {
        BindingTable::new(
            BindingTableSchema { columns: vec![] },
            vec![Binding::empty()],
        )
    };
    execute_pipeline(&plan.pipeline, input, &mut ctx).expect("read pipeline executes")
}

#[test]
fn read_executes_match_filter_project() {
    let table = execute_read("MATCH (n:Person) FILTER n.age >= 40 RETURN n.name AS name");

    assert_eq!(
        column_values(&table, "name"),
        vec![
            Value::String(exec_common::db_string("Bob")),
            Value::String(exec_common::db_string("Cara")),
        ]
    );
}

#[test]
fn read_executes_with_filter_chain() {
    let table = execute_read("MATCH (n:Person) WITH n FILTER n.age > 40 RETURN n.name AS name");

    assert_eq!(
        column_values(&table, "name"),
        vec![
            Value::String(exec_common::db_string("Bob")),
            Value::String(exec_common::db_string("Cara")),
        ]
    );
}

#[test]
fn read_executes_let_then_return() {
    let table = execute_read("MATCH (n:Person) LET doubled = n.age + n.age RETURN doubled");

    assert_eq!(
        column_values(&table, "doubled"),
        vec![Value::Int(60), Value::Int(84), Value::Int(110)]
    );
}

#[test]
fn read_executes_unwind_only_query() {
    let table = execute_read("UNWIND [1, 2] AS x RETURN x");

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Int(1), Value::Int(2)]
    );
}

#[test]
fn read_executes_distinct_projection() {
    let table = execute_read("MATCH (n:Person) RETURN DISTINCT n.tenant AS tenant");

    assert_eq!(
        column_values(&table, "tenant"),
        vec![
            Value::String(exec_common::db_string("t1")),
            Value::String(exec_common::db_string("t2")),
        ]
    );
}

#[test]
fn read_executes_limit_with_offset() {
    let table = execute_read("MATCH (n:Person) RETURN n.name AS name LIMIT 2 OFFSET 1");

    assert_eq!(
        column_values(&table, "name"),
        vec![
            Value::String(exec_common::db_string("Bob")),
            Value::String(exec_common::db_string("Cara")),
        ]
    );
}

#[test]
fn stored_edges_are_directed_for_is_directed() {
    let table = execute_read("MATCH ()-[e:KNOWS]->() RETURN e IS DIRECTED AS \"directed\"");

    assert_eq!(
        column_values(&table, "directed"),
        vec![Value::Bool(true), Value::Bool(true)]
    );
}

#[test]
fn stored_elements_match_is_labeled() {
    let nodes = execute_read("MATCH (n:Person) RETURN n IS LABELED :Person AS \"labeled\"");
    assert_eq!(
        column_values(&nodes, "labeled"),
        vec![Value::Bool(true), Value::Bool(true), Value::Bool(true)]
    );

    let edges = execute_read("MATCH ()-[e:KNOWS]->() RETURN e IS LABELED :KNOWS AS \"labeled\"");
    assert_eq!(
        column_values(&edges, "labeled"),
        vec![Value::Bool(true), Value::Bool(true)]
    );
}

#[test]
fn read_executes_projection_without_pattern() {
    let table = execute_read("RETURN 1 AS n");

    assert_eq!(column_values(&table, "n"), vec![Value::Int(1)]);
}

#[test]
fn read_executes_boolean_ordering() {
    let table = execute_read("RETURN false < true AS r");

    assert_eq!(column_values(&table, "r"), vec![Value::Bool(true)]);
}

// PARSE-01: `UNKNOWN` is the ISO §21.2 boolean unknown literal; it parses to
// `Literal::Null` and executes end-to-end to `Value::Null`.
#[test]
fn read_executes_unknown_literal_to_null() {
    let table = execute_read("RETURN UNKNOWN AS u");

    assert_eq!(column_values(&table, "u"), vec![Value::Null]);
}

// ANALYZE-01: NULL operands in comparison / arithmetic / boolean / unary
// operators analyze without error AND execute to NULL under three-valued logic.
#[test]
fn read_executes_null_operands_to_null() {
    for source in [
        "RETURN NULL < 5 AS r",
        "RETURN NULL + 1 AS r",
        "RETURN -NULL AS r",
        "RETURN NULL AND true AS r",
        "RETURN NOT NULL AS r",
    ] {
        let table = execute_read(source);
        assert_eq!(
            column_values(&table, "r"),
            vec![Value::Null],
            "{source} must execute to NULL"
        );
    }
}
