//! Source/destination predicate execution tests.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, execute_read};
use selene_core::Value;

#[test]
fn source_destination_predicates_use_node_left_and_edge_right() {
    let table = execute_read(
        "MATCH (a:Person)-[e]->(b:Person) \
         RETURN a IS SOURCE OF e AS source, \
                b IS DESTINATION OF e AS destination, \
                b IS SOURCE OF e AS wrong_source, \
                a IS DESTINATION OF e AS wrong_destination",
    );

    assert_eq!(column_values(&table, "source"), vec![Value::Bool(true)]);
    assert_eq!(
        column_values(&table, "destination"),
        vec![Value::Bool(true)]
    );
    assert_eq!(
        column_values(&table, "wrong_source"),
        vec![Value::Bool(false)]
    );
    assert_eq!(
        column_values(&table, "wrong_destination"),
        vec![Value::Bool(false)]
    );
}
