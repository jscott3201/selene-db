//! Pipeline aggregate executor tests.

mod exec_common;

use selene_core::Value;
use selene_gql::ExecutorError;

use exec_common::{column_values, execute_read, execute_read_result};

#[test]
fn avg_empty_group_returns_null_not_division_by_zero() {
    let table = execute_read("MATCH (n:Missing) RETURN avg(n.age) AS a");

    assert_eq!(column_values(&table, "a"), vec![Value::Null]);
}

#[test]
fn avg_skips_null_inputs() {
    let table = execute_read("UNWIND [1, NULL, 3] AS x RETURN avg(x) AS a");

    assert_eq!(column_values(&table, "a"), vec![Value::Float(2.0)]);
}

#[test]
fn avg_preserves_integer_precision_when_inputs_are_int() {
    let table = execute_read("UNWIND [2, 4] AS x RETURN avg(x) AS a");

    assert_eq!(column_values(&table, "a"), vec![Value::Float(3.0)]);
}

#[test]
fn sum_empty_returns_int_zero_not_null() {
    let table = execute_read("MATCH (n:Missing) RETURN sum(n.age) AS s");

    assert_eq!(column_values(&table, "s"), vec![Value::Int(0)]);
}

#[test]
fn sum_skips_null_inputs() {
    let table = execute_read("UNWIND [1, NULL, 3] AS x RETURN sum(x) AS s");

    assert_eq!(column_values(&table, "s"), vec![Value::Int(4)]);
}

#[test]
fn sum_overflow_returns_data_exception() {
    let err = execute_read_result("UNWIND [9223372036854775807, 1] AS x RETURN sum(x) AS s")
        .expect_err("sum overflow errors");

    assert!(matches!(err, ExecutorError::DataException { .. }));
}

#[test]
fn count_skips_null_inputs() {
    let table = execute_read("UNWIND [1, NULL, 3] AS x RETURN count(x) AS c");

    assert_eq!(column_values(&table, "c"), vec![Value::Int(2)]);
}

#[test]
fn count_empty_returns_zero() {
    let table = execute_read("MATCH (n:Missing) RETURN count(n.age) AS c");

    assert_eq!(column_values(&table, "c"), vec![Value::Int(0)]);
}

#[test]
fn count_star_includes_null_rows() {
    let table = execute_read("UNWIND [1, NULL, 3] AS x RETURN count(*) AS c");

    assert_eq!(column_values(&table, "c"), vec![Value::Int(3)]);
}

#[test]
fn min_max_empty_returns_null() {
    let table = execute_read("MATCH (n:Missing) RETURN min(n.age) AS mn, max(n.age) AS mx");

    assert_eq!(column_values(&table, "mn"), vec![Value::Null]);
    assert_eq!(column_values(&table, "mx"), vec![Value::Null]);
}

#[test]
fn min_max_skip_null_inputs() {
    let table = execute_read("UNWIND [2, NULL, 4, 1] AS x RETURN min(x) AS mn, max(x) AS mx");

    assert_eq!(column_values(&table, "mn"), vec![Value::Int(1)]);
    assert_eq!(column_values(&table, "mx"), vec![Value::Int(4)]);
}

#[test]
fn collect_preserves_input_order() {
    let table = execute_read("UNWIND [3, 1, 2] AS x RETURN collect(x) AS xs");

    assert_eq!(
        column_values(&table, "xs"),
        vec![Value::List(vec![
            Value::Int(3),
            Value::Int(1),
            Value::Int(2)
        ])]
    );
}

#[test]
fn collect_includes_nulls() {
    let table = execute_read("UNWIND [1, NULL, 3] AS x RETURN collect(x) AS xs");

    assert_eq!(
        column_values(&table, "xs"),
        vec![Value::List(vec![Value::Int(1), Value::Null, Value::Int(3)])]
    );
}

#[test]
fn collect_empty_returns_empty_list() {
    let table = execute_read("MATCH (n:Missing) RETURN collect(n.age) AS xs");

    assert_eq!(column_values(&table, "xs"), vec![Value::List(Vec::new())]);
}
