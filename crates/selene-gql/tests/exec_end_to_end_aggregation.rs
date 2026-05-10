//! End-to-end aggregation execution tests.

mod exec_common;

use selene_core::Value;

use exec_common::{column_values, execute_read};

#[test]
fn match_return_count_star_over_all_persons() {
    let table = execute_read("MATCH (n:Person) RETURN count(*) AS c");

    assert_eq!(column_values(&table, "c"), vec![Value::Int(3)]);
}

#[test]
fn match_with_property_group_by_label_count_per_label() {
    let table = execute_read(
        "MATCH (n:Person) RETURN n.tenant AS tenant, count(*) AS c \
         GROUP BY n.tenant ORDER BY tenant",
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
fn match_with_group_by_having_filters_on_aggregate() {
    let table = execute_read(
        "MATCH (n:Person) RETURN n.tenant AS tenant, count(*) AS c \
         GROUP BY n.tenant HAVING count(*) > 1",
    );

    assert_eq!(
        column_values(&table, "tenant"),
        vec![Value::String(exec_common::istr("t1"))]
    );
    assert_eq!(column_values(&table, "c"), vec![Value::Int(2)]);
}

#[test]
fn match_return_avg_with_null_handling() {
    let table = execute_read("MATCH (n:Person) RETURN avg(n.age) AS a");

    assert_eq!(column_values(&table, "a"), vec![Value::Float(127.0 / 3.0)]);
}

#[test]
fn match_with_let_then_group_by_computed_key() {
    let table = execute_read(
        "MATCH (n:Person) LET senior = n.age > 40 \
         RETURN senior AS senior, count(*) AS c GROUP BY senior ORDER BY senior",
    );

    assert_eq!(
        column_values(&table, "senior"),
        vec![Value::Bool(false), Value::Bool(true)]
    );
    assert_eq!(
        column_values(&table, "c"),
        vec![Value::Int(1), Value::Int(2)]
    );
}
