//! Pipeline OrderBy executor tests.

mod exec_common;

use selene_core::Value;

use exec_common::{column_values, execute_read, node_ids_for};

#[test]
fn order_by_single_key_ascending() {
    let table = execute_read("UNWIND [3, 1, 2] AS x RETURN x ORDER BY x");

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Int(1), Value::Int(2), Value::Int(3)]
    );
}

#[test]
fn order_by_single_key_descending() {
    let table = execute_read("UNWIND [3, 1, 2] AS x RETURN x ORDER BY x DESC");

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Int(3), Value::Int(2), Value::Int(1)]
    );
}

#[test]
fn order_by_multi_key_with_first_key_ties_breaks_on_second() {
    let table = execute_read("MATCH (n:Person) RETURN * ORDER BY n.tenant ASC, n.score DESC");

    assert_eq!(node_ids_for(&table, "n"), vec![Some(1), Some(2), Some(3)]);
}

#[test]
fn order_by_is_stable_for_equal_keys() {
    let table = execute_read("MATCH (n:Person) RETURN * ORDER BY n.tenant");

    assert_eq!(node_ids_for(&table, "n"), vec![Some(1), Some(2), Some(3)]);
}

#[test]
fn order_by_nulls_last_under_ascending() {
    let table = execute_read("UNWIND [2, NULL, 1] AS x RETURN x ORDER BY x");

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Int(1), Value::Int(2), Value::Null]
    );
}

#[test]
fn order_by_nulls_first_under_descending() {
    let table = execute_read("UNWIND [2, NULL, 1] AS x RETURN x ORDER BY x DESC");

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Null, Value::Int(2), Value::Int(1)]
    );
}
