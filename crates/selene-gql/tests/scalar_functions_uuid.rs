//! BRIEF-135c implementation-defined UUID scalar function coverage.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, execute_read, execute_read_result, first_scan, istr, planned};
use selene_core::{GraphId, PropertyValueType, Value, feature_register::FeatureId};
use selene_gql::{
    EmptyProcedureRegistry, IndexKind, Literal, OptimizeContext, ScanAccess, Session,
    StatementOutput, TypedIndexBounds, feature_walk, optimize, parse,
};
use selene_graph::{GraphTypeDef, SharedGraph, TypedIndexKind};
use selene_testing::MockIndexCatalog;

const UUID_TEXT: &str = "018f1b6d-7b89-7cc0-9f40-2c6f8d4df101";

fn single_value(source: &str, column: &str) -> Value {
    let table = execute_read(source);
    let mut values = column_values(&table, column);
    assert_eq!(values.len(), 1);
    values.pop().expect("one row")
}

fn uuid_value(source: &str) -> uuid::Uuid {
    match single_value(source, "value") {
        Value::Uuid(value) => value,
        other => panic!("expected UUID value, got {other:?}"),
    }
}

fn assert_status(source: &str, expected: &str) {
    let err = execute_read_result(source).expect_err("query should fail");
    assert_eq!(err.gqlstatus().as_str(), expected, "source: {source}");
}

fn assert_feature_recorded(source: &str) {
    let statement = parse(source).expect(source);
    let observed = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();
    assert!(
        observed.contains(&FeatureId::IM_UUID),
        "{source} should record IM_UUID, observed {observed:?}"
    );
}

fn string_value(value: Value) -> String {
    let Value::String(value) = value else {
        panic!("expected String, got {value:?}");
    };
    value.to_string()
}

fn rows_from_output(output: StatementOutput) -> selene_gql::BindingTable {
    let StatementOutput::Rows(table) = output else {
        panic!("expected row output");
    };
    table
}

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: istr("uuid.test.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .expect("graph type binds")
        .build()
        .expect("graph builds")
}

#[test]
fn uuid_v4_returns_random_version_uuid() {
    let value = uuid_value("RETURN uuid_v4() AS value");
    assert_eq!(value.get_version(), Some(uuid::Version::Random));
}

#[test]
fn uuid_v7_returns_sortable_random_version_uuid() {
    let value = uuid_value("RETURN uuid_v7() AS value");
    assert_eq!(value.get_version(), Some(uuid::Version::SortRand));
}

#[test]
fn uuid_function_parses_hyphenated_string() {
    assert_eq!(
        uuid_value(&format!("RETURN uuid('{UUID_TEXT}') AS value")),
        uuid::Uuid::parse_str(UUID_TEXT).expect("test UUID parses")
    );
}

#[test]
fn uuid_function_propagates_null() {
    assert_eq!(
        single_value("RETURN uuid(null) AS value", "value"),
        Value::Null
    );
}

#[test]
fn uuid_function_rejects_invalid_and_non_string_arguments() {
    assert_status("RETURN uuid('not-a-uuid') AS value", "22G03");
    assert_status("RETURN uuid(7) AS value", "22G03");
}

#[test]
fn uuid_functions_reject_wrong_arity() {
    for source in [
        "RETURN uuid_v4(1) AS value",
        "RETURN uuid_v7(1) AS value",
        "RETURN uuid() AS value",
        "RETURN uuid('a', 'b') AS value",
    ] {
        assert_status(source, "22G03");
    }
}

#[test]
fn uuid_functions_are_flagged_as_implementation_defined() {
    for source in [
        "RETURN uuid_v4() AS value",
        "RETURN uuid_v7() AS value",
        "RETURN uuid('018f1b6d-7b89-7cc0-9f40-2c6f8d4df101') AS value",
        "RETURN CAST('018f1b6d-7b89-7cc0-9f40-2c6f8d4df101' AS UUID) AS value",
    ] {
        assert_feature_recorded(source);
    }
}

#[test]
fn uuid_literals_and_type_names_are_flagged_as_implementation_defined() {
    for source in [
        "RETURN UUID '018f1b6d-7b89-7cc0-9f40-2c6f8d4df101' AS value",
        "RETURN null IS TYPED UUID AS value",
    ] {
        assert_feature_recorded(source);
    }
}

#[test]
fn uuid_literal_executes_as_uuid_value() {
    assert_eq!(
        uuid_value("RETURN UUID '018f1b6d-7b89-7cc0-9f40-2c6f8d4df101' AS value"),
        uuid::Uuid::parse_str(UUID_TEXT).expect("test UUID parses")
    );
}

#[test]
fn is_typed_uuid_matches_uuid_values() {
    assert_eq!(
        single_value(
            "RETURN UUID '018f1b6d-7b89-7cc0-9f40-2c6f8d4df101' IS TYPED UUID AS value",
            "value",
        ),
        Value::Bool(true)
    );
}

#[test]
fn is_typed_uuid_rejects_non_uuid_values() {
    assert_eq!(
        single_value(
            "RETURN '018f1b6d-7b89-7cc0-9f40-2c6f8d4df101' IS TYPED UUID AS value",
            "value",
        ),
        Value::Bool(false)
    );
}

#[test]
fn cast_string_as_uuid_returns_uuid_value() {
    assert_eq!(
        uuid_value(&format!("RETURN CAST('{UUID_TEXT}' AS UUID) AS value")),
        uuid::Uuid::parse_str(UUID_TEXT).expect("test UUID parses")
    );
}

#[test]
fn cast_computed_string_as_uuid_returns_uuid_value() {
    assert_eq!(
        uuid_value(&format!(
            "RETURN CAST(upper('{UUID_TEXT}') AS UUID) AS value"
        )),
        uuid::Uuid::parse_str(UUID_TEXT).expect("test UUID parses")
    );
}

#[test]
fn cast_uuid_as_string_returns_string() {
    assert_eq!(
        string_value(single_value(
            "RETURN CAST(UUID '018f1b6d-7b89-7cc0-9f40-2c6f8d4df101' AS STRING) AS value",
            "value",
        )),
        UUID_TEXT
    );
}

#[test]
fn cast_uuid_as_uuid_preserves_value() {
    assert_eq!(
        uuid_value("RETURN CAST(UUID '018f1b6d-7b89-7cc0-9f40-2c6f8d4df101' AS UUID) AS value"),
        uuid::Uuid::parse_str(UUID_TEXT).expect("test UUID parses")
    );
}

#[test]
fn cast_null_as_uuid_propagates_null() {
    assert_eq!(
        single_value("RETURN CAST(null AS UUID) AS value", "value"),
        Value::Null
    );
}

#[test]
fn cast_invalid_uuid_string_returns_22g03() {
    assert_status("RETURN CAST('not-a-uuid' AS UUID) AS value", "22G03");
}

#[test]
fn cast_non_string_as_uuid_returns_42n01() {
    assert_status("RETURN CAST(7 AS UUID) AS value", "42N01");
}

#[test]
fn cast_uuid_to_unsupported_target_returns_42n01() {
    assert_status(
        "RETURN CAST(UUID '018f1b6d-7b89-7cc0-9f40-2c6f8d4df101' AS INTEGER) AS value",
        "42N01",
    );
}

#[test]
fn planner_routes_uuid_literal_equality_to_uuid_typed_index() {
    let label = istr("Thing");
    let property = istr("id");
    let expected = uuid::Uuid::parse_str(UUID_TEXT).expect("test UUID parses");
    let catalog = MockIndexCatalog::new().with_node_typed_index(label, property, IndexKind::Uuid);
    let plan = planned(&format!(
        "MATCH (n:Thing) WHERE n.id = UUID '{UUID_TEXT}' RETURN n"
    ));
    let ctx = OptimizeContext::default().with_index_catalog(&catalog);
    let plan = optimize(plan, &ctx);
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let scan = first_scan(&pattern.join_tree).expect("scan");

    let ScanAccess::TypedIndexRange { kind, bounds, .. } = &scan.access else {
        panic!("expected UUID typed-index access, got {:?}", scan.access);
    };
    assert_eq!(*kind, IndexKind::Uuid);
    assert!(
        matches!(
            bounds,
            TypedIndexBounds::Equality(selene_gql::IndexKey::Literal(Literal::Uuid(value, _)))
                if *value == expected
        ),
        "expected UUID equality bounds, got {bounds:?}"
    );
}

#[test]
fn uuid_indexed_node_type_round_trips_catalog_and_execution() {
    let graph = empty_closed_graph(13_504);
    let mut session = Session::new(&graph);
    session
        .execute_source(
            "CREATE NODE TYPE :Thing (id :: UUID INDEXED)",
            &EmptyProcedureRegistry,
        )
        .expect("UUID node type creates");

    let label = istr("Thing");
    let property = istr("id");
    let graph_type = graph.graph_type().expect("graph has type");
    let declaration = &graph_type.node_types[0].properties[0];
    assert_eq!(declaration.value_type, PropertyValueType::Uuid);
    assert_eq!(
        graph
            .read()
            .property_index_for(&label, &property)
            .expect("UUID property index exists")
            .kind(),
        TypedIndexKind::Uuid
    );

    session
        .execute_source(
            "INSERT (:Thing {id: UUID '018f1b6d-7b89-7cc0-9f40-2c6f8d4df101'})",
            &EmptyProcedureRegistry,
        )
        .expect("UUID property inserts");
    let output = session
        .execute_source(
            "MATCH (n:Thing) WHERE n.id = UUID '018f1b6d-7b89-7cc0-9f40-2c6f8d4df101' \
             RETURN n.id AS value",
            &EmptyProcedureRegistry,
        )
        .expect("UUID typed-index lookup executes");
    let table = rows_from_output(output);
    let values = column_values(&table, "value");
    assert_eq!(
        values,
        vec![Value::Uuid(
            uuid::Uuid::parse_str(UUID_TEXT).expect("test UUID parses")
        )]
    );
}

#[test]
fn uuid_default_literal_is_rejected_until_default_literals_are_expanded() {
    let graph = empty_closed_graph(13_505);
    let mut session = Session::new(&graph);
    let err = session
        .execute_source(
            "CREATE NODE TYPE :Thing (id :: UUID DEFAULT UUID '018f1b6d-7b89-7cc0-9f40-2c6f8d4df101')",
            &EmptyProcedureRegistry,
        )
        .expect_err("UUID DEFAULT literals are deferred");
    assert_eq!(err.gqlstatus().as_str(), "42N01");
}
