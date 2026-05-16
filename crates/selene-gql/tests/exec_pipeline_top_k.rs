//! Pipeline TopK executor tests.

mod exec_common;

use selene_core::Value;

use exec_common::{column_values, execute_optimized_read, execute_read};

#[test]
fn top_k_returns_same_rows_as_order_by_then_limit() {
    let source = "UNWIND [5, 1, 4, 2, 3] AS x RETURN x ORDER BY x LIMIT 3 OFFSET 1";

    assert_eq!(
        column_values(&execute_optimized_read(source), "x"),
        column_values(&execute_read(source), "x")
    );
}

#[test]
fn top_k_with_offset_zero_count_n_returns_top_n() {
    let table = execute_optimized_read("UNWIND [5, 1, 4, 2, 3] AS x RETURN x ORDER BY x LIMIT 2");

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Int(1), Value::Int(2)]
    );
}

#[test]
fn top_k_orders_by_pre_projection_binding_before_project() {
    let table = execute_optimized_read(
        "MATCH (n:Person) FILTER n.age >= 21 RETURN n.name AS name ORDER BY n.age DESC LIMIT 2",
    );

    assert_eq!(
        column_values(&table, "name"),
        vec![
            Value::String(exec_common::istr("Cara")),
            Value::String(exec_common::istr("Bob")),
        ]
    );
}

#[test]
fn top_k_with_offset_n_count_m_returns_n_to_n_plus_m_in_order() {
    let table =
        execute_optimized_read("UNWIND [5, 1, 4, 2, 3] AS x RETURN x ORDER BY x LIMIT 2 OFFSET 2");

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Int(3), Value::Int(4)]
    );
}

#[test]
fn top_k_with_offset_beyond_input_returns_empty() {
    let table =
        execute_optimized_read("UNWIND [5, 1, 4, 2, 3] AS x RETURN x ORDER BY x LIMIT 2 OFFSET 99");

    assert!(table.is_empty());
}

#[test]
fn top_k_count_zero_returns_empty() {
    let table = execute_optimized_read("UNWIND [5, 1, 4] AS x RETURN x ORDER BY x LIMIT 0");

    assert!(table.is_empty());
}
