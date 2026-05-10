//! Pipeline GroupBy executor tests.

mod exec_common;

use selene_core::Value;

use exec_common::{LARGE_COUNTER_A, LARGE_COUNTER_B, column_values, execute_read};

#[test]
fn group_by_single_key_partitions_correctly() {
    let table =
        execute_read("UNWIND [1, 2, 1] AS x RETURN x AS x, count(*) AS c GROUP BY x ORDER BY x");

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Int(1), Value::Int(2)]
    );
    assert_eq!(
        column_values(&table, "c"),
        vec![Value::Int(2), Value::Int(1)]
    );
}

#[test]
fn group_by_multi_key_partitions_by_tuple() {
    let table = execute_read(
        "MATCH (n:Person) RETURN n.tenant AS tenant, n.kind AS kind, count(*) AS c \
         GROUP BY n.tenant, n.kind ORDER BY tenant",
    );

    assert_eq!(
        column_values(&table, "tenant"),
        vec![
            Value::String(exec_common::istr("t1")),
            Value::String(exec_common::istr("t2")),
        ]
    );
    assert_eq!(
        column_values(&table, "c"),
        vec![Value::Int(2), Value::Int(1)]
    );
}

#[test]
fn group_by_implicit_with_empty_input_emits_one_row_with_empty_aggregates() {
    let table = execute_read("MATCH (n:Missing) RETURN count(*) AS c, sum(n.age) AS s");

    assert_eq!(table.row_count(), 1);
    assert_eq!(column_values(&table, "c"), vec![Value::Int(0)]);
    assert_eq!(column_values(&table, "s"), vec![Value::Int(0)]);
}

#[test]
fn group_by_explicit_with_empty_input_emits_no_rows() {
    let table = execute_read("MATCH (n:Missing) RETURN n.age AS age, count(*) AS c GROUP BY n.age");

    assert!(table.is_empty());
}

#[test]
fn group_by_uses_lossless_numeric_equality_for_keys() {
    let table = execute_read(
        "UNWIND [9007199254740992, 9007199254740993, 9007199254740992.0] AS x \
         RETURN x AS x, count(*) AS c GROUP BY x ORDER BY c DESC",
    );

    assert_eq!(
        column_values(&table, "c"),
        vec![Value::Int(2), Value::Int(1)]
    );
    assert!(column_values(&table, "x").contains(&Value::Int(LARGE_COUNTER_B)));
    assert!(column_values(&table, "x").contains(&Value::Int(LARGE_COUNTER_A)));
}

#[test]
fn group_by_handles_null_keys_as_distinct_group() {
    let table = execute_read(
        "UNWIND [NULL, NULL, 1] AS x RETURN x AS x, count(*) AS c GROUP BY x ORDER BY x",
    );

    assert_eq!(column_values(&table, "x"), vec![Value::Int(1), Value::Null]);
    assert_eq!(
        column_values(&table, "c"),
        vec![Value::Int(1), Value::Int(2)]
    );
}

#[test]
fn group_by_preserves_first_row_emission_order() {
    let table = execute_read("UNWIND [2, 1, 2, 3, 1] AS x RETURN x AS x, count(*) AS c GROUP BY x");

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Int(2), Value::Int(1), Value::Int(3)]
    );
}
