//! BRIEF-135b-2 ISO identity and length scalar function coverage.

#![cfg(feature = "test-harness")]

mod exec_common;

use std::sync::Arc;

use exec_common::{column_values, db_string, execute_read, execute_read_result};
use selene_core::{
    DbString, EdgeDirection, EdgeId, GraphId, NodeId, Path, PathSegment, Value,
    feature_register::FeatureId,
};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, GqlType, ProcedureContext, ProcedureError,
    ProcedureHandle, ProcedureMetadata, ProcedureMutability, ProcedureOutputColumn,
    ProcedureOutputSchema, ProcedureParameter, ProcedureRegistry, ProcedureResult,
    ProcedureSignature, ProcedureTier, Session, StatementOutput, feature_walk, parse,
};
use selene_graph::SharedGraph;
use smallvec::smallvec;

const BINDING_TABLE_WITH_ROWS: ProcedureHandle = ProcedureHandle::new(1);

fn single_value(source: &str, column: &str) -> Value {
    let table = execute_read(source);
    let mut values = column_values(&table, column);
    assert_eq!(values.len(), 1);
    values.pop().expect("one row")
}

fn assert_status(source: &str, expected: &str) {
    let err = execute_read_result(source).expect_err("query should fail");
    assert_eq!(err.gqlstatus().as_str(), expected, "source: {source}");
}

fn assert_external_id(value: Value, prefix: &str) {
    let Value::String(actual) = value else {
        panic!("expected String ID, got {value:?}");
    };
    assert!(
        actual.as_str().starts_with(prefix),
        "expected {prefix} ID, got {actual}"
    );
}

#[derive(Debug)]
struct BindingTableFixtureRegistry;

impl ProcedureRegistry for BindingTableFixtureRegistry {
    fn lookup(&self, name: &[DbString]) -> Option<ProcedureMetadata> {
        if name.len() != 2
            || name[0].as_str() != "selene_test"
            || name[1].as_str() != "binding_table_with_rows"
        {
            return None;
        }
        Some(ProcedureMetadata::new(
            BINDING_TABLE_WITH_ROWS,
            ProcedureSignature::new(vec![ProcedureParameter::new(
                db_string("rows"),
                GqlType::Integer,
                false,
            )]),
            ProcedureOutputSchema {
                columns: vec![ProcedureOutputColumn::new(
                    db_string("t"),
                    GqlType::TableRef,
                )],
            },
            ProcedureTier::Graph,
            ProcedureMutability::Read,
        ))
    }

    fn execute(
        &self,
        handle: ProcedureHandle,
        args: &[Value],
        ctx: &mut ProcedureContext<'_, '_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        assert_eq!(handle, BINDING_TABLE_WITH_ROWS);
        let Some(Value::Int(rows)) = args.first() else {
            return Err(ProcedureError::Internal {
                detail: "fixture expects integer row count".to_owned(),
            });
        };
        let rows = usize::try_from(*rows).map_err(|_| ProcedureError::Internal {
            detail: "fixture row count must be non-negative".to_owned(),
        })?;
        let table = table_with_rows(rows);
        let id = ctx.register_binding_table(Arc::new(table));
        Ok(ProcedureResult {
            rows: vec![vec![Value::TableRef(id)]],
        })
    }
}

fn table_with_rows(rows: usize) -> BindingTable {
    BindingTable::new(
        BindingTableSchema {
            columns: Vec::new(),
        },
        (0..rows).map(|_| Binding::empty()).collect(),
    )
}

fn execute_with_registry(source: &str, registry: &dyn ProcedureRegistry) -> BindingTable {
    let graph = SharedGraph::new(GraphId::new(9200));
    let mut session = Session::new(&graph);
    let output = session
        .execute_source(source, registry)
        .expect("source executes");
    rows_from_output(output)
}

fn rows_from_output(output: StatementOutput) -> BindingTable {
    let StatementOutput::Rows(table) = output else {
        panic!("RETURN should produce rows");
    };
    table
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

#[test]
fn element_id_returns_string_for_nodes_and_edges() {
    assert_external_id(
        single_value("MATCH (n:Person) RETURN element_id(n) AS id LIMIT 1", "id"),
        "NodeId(",
    );
    assert_external_id(
        single_value("MATCH ()-[e]->() RETURN element_id(e) AS id LIMIT 1", "id"),
        "EdgeId(",
    );
}

#[test]
fn element_id_propagates_null() {
    assert_eq!(
        single_value("RETURN element_id(null) AS id", "id"),
        Value::Null
    );
}

#[test]
fn element_id_rejects_non_element_arguments() {
    for source in [
        "RETURN element_id(1) AS id",
        "RETURN element_id('x') AS id",
        "MATCH (n:Person) RETURN element_id([n]) AS id LIMIT 1",
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn element_id_rejects_wrong_arity() {
    assert_status("RETURN element_id() AS id", "22G03");
    assert_status("RETURN element_id(1, 2) AS id", "22G03");
}

#[test]
fn element_id_is_stable_for_equality_within_statement() {
    assert_eq!(
        single_value(
            "MATCH (n:Person) RETURN element_id(n) = element_id(n) AS same LIMIT 1",
            "same",
        ),
        Value::Bool(true)
    );
}

#[test]
fn element_id_records_g100_feature() {
    let statement = parse("MATCH (n) RETURN element_id(n) AS id").expect("source parses");
    let features = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    assert!(
        features.contains(&FeatureId::G100),
        "element_id should record G100, observed {features:?}"
    );
}

#[test]
fn cardinality_counts_lists_paths_and_records() {
    let cases = [
        ("RETURN cardinality([1, 2, 3]) AS value", Value::Int(3)),
        ("RETURN cardinality([]) AS value", Value::Int(0)),
        ("RETURN cardinality({a: 1, b: 2}) AS value", Value::Int(2)),
    ];

    for (source, expected) in cases {
        assert_eq!(single_value(source, "value"), expected, "source: {source}");
    }
}

#[test]
fn cardinality_counts_path_parameter() {
    let graph = SharedGraph::new(GraphId::new(9202));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("p"), two_edge_path_value());

    let table = rows_from_output(
        session
            .execute_source(
                "RETURN cardinality($p) AS value",
                &BindingTableFixtureRegistry,
            )
            .expect("source executes"),
    );

    assert_eq!(column_values(&table, "value"), vec![Value::Int(5)]);
}

#[test]
fn cardinality_counts_session_table_parameter() {
    let graph = SharedGraph::new(GraphId::new(9201));
    let mut session = Session::new(&graph);
    session.bind_table_parameter(db_string("t"), table_with_rows(5));

    let output = session
        .execute_source(
            "RETURN cardinality($t) AS value",
            &BindingTableFixtureRegistry,
        )
        .expect("source executes");
    let table = rows_from_output(output);

    assert_eq!(column_values(&table, "value"), vec![Value::Int(5)]);
}

#[test]
fn cardinality_counts_call_yield_table_ref() {
    let table = execute_with_registry(
        "UNWIND [0] AS seed CALL selene_test.binding_table_with_rows(7) YIELD t RETURN cardinality(t) AS value",
        &BindingTableFixtureRegistry,
    );

    assert_eq!(column_values(&table, "value"), vec![Value::Int(7)]);
}

#[test]
fn cardinality_propagates_null_and_rejects_invalid_values() {
    assert_eq!(
        single_value("RETURN cardinality(null) AS value", "value"),
        Value::Null
    );

    for source in [
        "RETURN cardinality(1) AS value",
        "RETURN cardinality('x') AS value",
        "MATCH (n:Person) RETURN cardinality(n) AS value LIMIT 1",
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn cardinality_rejects_wrong_arity() {
    assert_status("RETURN cardinality() AS value", "22G03");
    assert_status("RETURN cardinality([1], [2]) AS value", "22G03");
}

#[test]
fn cardinality_records_gf12_feature() {
    let statement = parse("RETURN cardinality([1, 2, 3]) AS value").expect("source parses");
    let features = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    assert!(
        features.contains(&FeatureId::GF12),
        "cardinality should record GF12, observed {features:?}"
    );
}

#[test]
fn path_length_counts_path_edges() {
    let graph = SharedGraph::new(GraphId::new(9203));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("p"), two_edge_path_value());
    session.bind_parameter(db_string("empty"), zero_edge_path_value());

    let table = rows_from_output(
        session
            .execute_source(
                "RETURN path_length($p) AS path_edges, path_length($empty) AS empty_edges",
                &BindingTableFixtureRegistry,
            )
            .expect("source executes"),
    );

    assert_eq!(column_values(&table, "path_edges"), vec![Value::Int(2)]);
    assert_eq!(column_values(&table, "empty_edges"), vec![Value::Int(0)]);
}

#[test]
fn path_length_propagates_null_and_rejects_invalid_values() {
    assert_eq!(
        single_value("RETURN path_length(null) AS value", "value"),
        Value::Null
    );

    for source in [
        "RETURN path_length(1) AS value",
        "RETURN path_length([1]) AS value",
        "MATCH (n:Person) RETURN path_length(n) AS value LIMIT 1",
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn path_length_rejects_wrong_arity() {
    assert_status("RETURN path_length() AS value", "22G03");
    assert_status("RETURN path_length(null, null) AS value", "22G03");
}

#[test]
fn path_length_records_gf04_feature() {
    let statement = parse("RETURN path_length(null) AS value").expect("source parses");
    let features = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    assert!(
        features.contains(&FeatureId::GF04),
        "path_length should record GF04, observed {features:?}"
    );
}

#[test]
fn char_length_counts_unicode_scalars_and_alias_matches() {
    let cases = [
        ("RETURN char_length('') AS value", Value::Int(0)),
        ("RETURN char_length('cafe') AS value", Value::Int(4)),
        ("RETURN char_length('café') AS value", Value::Int(4)),
        ("RETURN char_length('日本') AS value", Value::Int(2)),
        ("RETURN CHARACTER_LENGTH('abc') AS value", Value::Int(3)),
    ];

    for (source, expected) in cases {
        assert_eq!(single_value(source, "value"), expected, "source: {source}");
    }

    assert_eq!(
        single_value("RETURN char_length('selene') AS value", "value"),
        single_value("RETURN character_length('selene') AS value", "value")
    );
}

#[test]
fn char_length_propagates_null_and_rejects_invalid_values() {
    assert_eq!(
        single_value("RETURN char_length(null) AS value", "value"),
        Value::Null
    );

    for source in [
        "RETURN char_length(1) AS value",
        "RETURN char_length([1]) AS value",
        "MATCH (n:Person) RETURN character_length(n) AS value LIMIT 1",
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn char_length_rejects_wrong_arity() {
    assert_status("RETURN char_length() AS value", "22G03");
    assert_status("RETURN character_length('a', 'b') AS value", "22G03");
}

#[test]
fn non_iso_length_function_is_not_in_the_scalar_set() {
    assert_status("RETURN length('abc') AS value", "22G03");
}

#[test]
fn char_length_has_no_optional_feature_attribution() {
    let statement = parse("RETURN char_length('abc') AS value").expect("source parses");
    let features = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    assert!(
        features.is_empty(),
        "char_length should not record an optional feature, observed {features:?}"
    );
}

#[test]
fn byte_length_counts_octets_and_alias_matches() {
    let cases = [
        ("RETURN byte_length(X'') AS value", Value::Int(0)),
        ("RETURN byte_length(X'CAFE00') AS value", Value::Int(3)),
        ("RETURN OCTET_LENGTH(X'CA FE') AS value", Value::Int(2)),
    ];

    for (source, expected) in cases {
        assert_eq!(single_value(source, "value"), expected, "source: {source}");
    }

    assert_eq!(
        single_value("RETURN byte_length(X'CAFE') AS value", "value"),
        single_value("RETURN octet_length(X'CAFE') AS value", "value")
    );
}

#[test]
fn byte_length_propagates_null_and_rejects_invalid_values() {
    assert_eq!(
        single_value("RETURN byte_length(null) AS value", "value"),
        Value::Null
    );

    for source in [
        "RETURN byte_length('café') AS value",
        "RETURN octet_length(1) AS value",
        "RETURN byte_length([X'CA']) AS value",
        "MATCH (n:Person) RETURN octet_length(n) AS value LIMIT 1",
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn byte_length_rejects_wrong_arity() {
    assert_status("RETURN byte_length() AS value", "22G03");
    assert_status("RETURN octet_length(X'CA', X'FE') AS value", "22G03");
}

#[test]
fn byte_length_has_no_optional_feature_attribution() {
    let statement = parse("RETURN octet_length(X'CAFE') AS value").expect("source parses");
    let features = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    assert!(
        features.is_empty(),
        "byte_length should not record an optional feature, observed {features:?}"
    );
}
