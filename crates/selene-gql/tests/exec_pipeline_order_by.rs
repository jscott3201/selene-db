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

#[test]
fn order_by_explicit_nulls_policy_is_direction_aware_across_all_four_combos() {
    // GQLRT-28: only the two DEFAULT null placements (ASC->last, DESC->first)
    // were tested. The explicit `NULLS FIRST/LAST` × `ASC/DESC` matrix exercises
    // the direction-aware flip in `order_by::null_sort_order` (under DESC the
    // comparator reverses, so the policy must be flipped to land NULLs where the
    // user asked). Assert the full row order for all four combinations.
    let source = "UNWIND [2, NULL, 1] AS x RETURN x ORDER BY x";

    // ASC: data ascends 1,2; NULLS FIRST puts NULL at the head, LAST at the tail.
    assert_eq!(
        column_values(&execute_read(&format!("{source} ASC NULLS FIRST")), "x"),
        vec![Value::Null, Value::Int(1), Value::Int(2)]
    );
    assert_eq!(
        column_values(&execute_read(&format!("{source} ASC NULLS LAST")), "x"),
        vec![Value::Int(1), Value::Int(2), Value::Null]
    );

    // DESC: data descends 2,1; the explicit policy still honors FIRST/LAST
    // literally despite the reversed comparator.
    assert_eq!(
        column_values(&execute_read(&format!("{source} DESC NULLS FIRST")), "x"),
        vec![Value::Null, Value::Int(2), Value::Int(1)]
    );
    assert_eq!(
        column_values(&execute_read(&format!("{source} DESC NULLS LAST")), "x"),
        vec![Value::Int(2), Value::Int(1), Value::Null]
    );
}

#[test]
fn order_by_list_values_sort_lexicographically() {
    // ISO §22.14 ordering: lists order element-wise by ascending ordinal; the
    // first differing element decides ([1,2] < [1,3] < [2,0]).
    let table = execute_read("UNWIND [[2, 0], [1, 3], [1, 2]] AS x RETURN x ORDER BY x");

    assert_eq!(
        column_values(&table, "x"),
        vec![
            Value::List(vec![Value::Int(1), Value::Int(2)]),
            Value::List(vec![Value::Int(1), Value::Int(3)]),
            Value::List(vec![Value::Int(2), Value::Int(0)]),
        ]
    );
}

#[test]
fn order_by_list_values_shorter_prefix_precedes() {
    // Cardinality tiebreak: on an equal prefix the shorter list sorts first.
    let table = execute_read("UNWIND [[1, 2, 3], [1, 2]] AS x RETURN x ORDER BY x");

    assert_eq!(
        column_values(&table, "x"),
        vec![
            Value::List(vec![Value::Int(1), Value::Int(2)]),
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        ]
    );
}

#[test]
fn order_by_list_values_descending_reverses_lexicographic_order() {
    let table = execute_read("UNWIND [[1, 2], [2, 0], [1, 3]] AS x RETURN x ORDER BY x DESC");

    assert_eq!(
        column_values(&table, "x"),
        vec![
            Value::List(vec![Value::Int(2), Value::Int(0)]),
            Value::List(vec![Value::Int(1), Value::Int(3)]),
            Value::List(vec![Value::Int(1), Value::Int(2)]),
        ]
    );
}
