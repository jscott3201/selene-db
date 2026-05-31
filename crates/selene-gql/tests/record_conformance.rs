//! RECORD field-name-equality conformance tests (GQLRT-14).
//!
//! ISO/IEC 39075:2024 §4.15 defines record equality via a field-name bijection,
//! not a positional zip: `{a:1, b:2}` equals `{b:2, a:1}`. These end-to-end
//! tests pin that `=` over record literals and DISTINCT / GROUP BY over permuted
//! records honour the bijection, reaching the runtime through record literals.

mod exec_common;

use selene_core::Value;

use exec_common::{column_values, execute_read};

#[test]
fn record_equality_ignores_field_order() {
    let table = execute_read("RETURN {a: 1, b: 2} = {b: 2, a: 1} AS eq");

    assert_eq!(column_values(&table, "eq"), vec![Value::Bool(true)]);
}

#[test]
fn record_inequality_on_differing_field_sets() {
    let table = execute_read("RETURN {a: 1} = {a: 1, b: 2} AS eq");

    assert_eq!(column_values(&table, "eq"), vec![Value::Bool(false)]);
}

#[test]
fn record_equality_with_null_field_is_unknown() {
    let table = execute_read("RETURN ({a: 1, b: null} = {b: null, a: 1}) IS NULL AS unknown");

    assert_eq!(column_values(&table, "unknown"), vec![Value::Bool(true)]);
}

#[test]
fn distinct_over_permuted_records_collapses() {
    let table = execute_read("UNWIND [{a: 1, b: 2}, {b: 2, a: 1}] AS r RETURN DISTINCT r");

    assert_eq!(
        table.row_count(),
        1,
        "permuted records are one DISTINCT value"
    );
}

#[test]
fn distinct_keeps_records_with_different_field_values_apart() {
    let table = execute_read("UNWIND [{a: 1, b: 2}, {a: 1, b: 3}] AS r RETURN DISTINCT r");

    assert_eq!(table.row_count(), 2);
}

#[test]
fn group_by_collapses_permuted_record_keys() {
    let table = execute_read(
        "UNWIND [{a: 1, b: 2}, {b: 2, a: 1}, {a: 9, b: 9}] AS r \
         RETURN r AS key, count(*) AS n GROUP BY r",
    );

    // The two permuted records share a group; the distinct record is its own.
    assert_eq!(table.row_count(), 2);
    let counts = column_values(&table, "n");
    assert!(counts.contains(&Value::Int(2)));
    assert!(counts.contains(&Value::Int(1)));
}
