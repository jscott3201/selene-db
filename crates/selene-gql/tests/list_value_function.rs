//! List value function coverage for ISO/IEC 39075:2024 section 20.16.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, db_string, execute_read, execute_read_result};
use selene_core::{
    EdgeDirection, EdgeId, GraphId, NodeId, Path, PathSegment, Value, feature_register::FeatureId,
};
use selene_gql::{EmptyProcedureRegistry, Session, StatementOutput, feature_walk, parse};
use selene_graph::SharedGraph;
use smallvec::smallvec;

fn single_value(source: &str, column: &str) -> Value {
    let table = execute_read(source);
    let mut values = column_values(&table, column);
    assert_eq!(values.len(), 1, "{source}");
    values.pop().expect("one row")
}

fn assert_status(source: &str, expected: &str) {
    let err = execute_read_result(source).expect_err("query should fail");
    assert_eq!(err.gqlstatus().as_str(), expected, "source: {source}");
}

fn two_edge_path_value() -> Value {
    Value::Path(Box::new(Path {
        graph: GraphId::new(1),
        start: NodeId::new(1),
        segments: smallvec![
            PathSegment {
                edge: EdgeId::new(1),
                direction: EdgeDirection::Outgoing,
                node: NodeId::new(2),
            },
            PathSegment {
                edge: EdgeId::new(2),
                direction: EdgeDirection::Outgoing,
                node: NodeId::new(3),
            },
        ],
    }))
}

fn zero_edge_path_value() -> Value {
    Value::Path(Box::new(Path {
        graph: GraphId::new(1),
        start: NodeId::new(1),
        segments: smallvec![],
    }))
}

fn rows_from_output(output: StatementOutput) -> selene_gql::BindingTable {
    let StatementOutput::Rows(table) = output else {
        panic!("RETURN should produce rows");
    };
    table
}

#[test]
fn trim_list_function_removes_tail_elements() {
    assert_eq!(
        single_value("RETURN trim([1, 2, 3, 4], 2) AS value", "value"),
        Value::List(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        single_value("RETURN trim([1, 2], 0) AS value", "value"),
        Value::List(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        single_value("RETURN trim([1, 2], 2) AS value", "value"),
        Value::List(vec![])
    );
}

#[test]
fn trim_list_function_propagates_nulls_in_iso_evaluation_order() {
    assert_eq!(
        single_value("RETURN trim(1 / 0, null) AS value", "value"),
        Value::Null
    );
    assert_eq!(
        single_value("RETURN trim(null, 0) AS value", "value"),
        Value::Null
    );
}

#[test]
fn trim_list_function_reports_list_element_errors() {
    assert_status("RETURN trim([1], -1) AS value", "22G0C");
    assert_status("RETURN trim([1], 2) AS value", "22G0C");
}

#[test]
fn trim_list_function_rejects_non_list_or_non_integer_arguments() {
    assert_status("RETURN trim('abc', 1) AS value", "22G03");
    assert_status("RETURN trim([1], 1.5) AS value", "22G03");
}

#[test]
fn trim_list_function_records_gv50() {
    let statement = parse("MATCH (n) RETURN trim(n.values, 1)").expect("source parses");
    let features = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    assert!(
        features.contains(&FeatureId::GV50),
        "trim list function should record GV50, observed {features:?}"
    );
}

#[test]
fn elements_function_returns_ordered_path_element_list() {
    let graph = SharedGraph::new(GraphId::new(9300));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("p"), two_edge_path_value());
    session.bind_parameter(db_string("empty"), zero_edge_path_value());

    let table = rows_from_output(
        session
            .execute_source(
                "RETURN elements($p) AS elements, elements($empty) AS empty_elements",
                &EmptyProcedureRegistry,
            )
            .expect("source executes"),
    );

    assert_eq!(
        column_values(&table, "elements"),
        vec![Value::List(vec![
            Value::NodeRef(NodeId::new(1)),
            Value::EdgeRef(EdgeId::new(1)),
            Value::NodeRef(NodeId::new(2)),
            Value::EdgeRef(EdgeId::new(2)),
            Value::NodeRef(NodeId::new(3)),
        ])]
    );
    assert_eq!(
        column_values(&table, "empty_elements"),
        vec![Value::List(vec![Value::NodeRef(NodeId::new(1))])]
    );
}

#[test]
fn elements_function_propagates_null_and_rejects_non_paths() {
    assert_eq!(
        single_value("RETURN elements(null) AS value", "value"),
        Value::Null
    );

    for source in [
        "RETURN elements(1) AS value",
        "RETURN elements([1]) AS value",
        "MATCH (n:Person) RETURN elements(n) AS value LIMIT 1",
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn elements_function_rejects_wrong_arity() {
    assert_status("RETURN elements() AS value", "22G03");
    assert_status("RETURN elements(null, null) AS value", "22G03");
}

#[test]
fn elements_function_records_gf04_and_gv50() {
    let statement = parse("MATCH p = (a)-[]->(b) RETURN elements(p)").expect("source parses");
    let features = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    assert!(
        features.contains(&FeatureId::GF04),
        "elements function should record GF04, observed {features:?}"
    );
    assert!(
        features.contains(&FeatureId::GV50),
        "elements function should record GV50, observed {features:?}"
    );
}
