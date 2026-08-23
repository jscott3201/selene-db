//! Regression coverage for first-class VECTOR value type support.

use selene_core::{GraphId, Value, VectorValue, db_string};
use selene_gql::{
    EmptyProcedureRegistry, GqlStatus, Session, StatementOutput, feature_walk, parse,
};
use selene_graph::SharedGraph;
use selene_profile::FeatureId;

fn first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(41_210));
    let mut session = Session::new(&graph);
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    let StatementOutput::Rows(table) = output else {
        panic!("`{source}` produced non-row output");
    };
    table.rows()[0].values()[0].clone()
}

fn first_status(source: &str) -> String {
    let graph = SharedGraph::new(GraphId::new(41_211));
    let mut session = Session::new(&graph);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

fn bound_status(source: &str, value: Value) -> String {
    let graph = SharedGraph::new(GraphId::new(41_212));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("value").expect("parameter name"), value);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

fn feature_ids(source: &str) -> Vec<FeatureId> {
    feature_walk(&parse(source).expect("source parses"))
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect()
}

#[test]
fn vector_type_name_parses_at_gql_type_boundaries() {
    for source in [
        "RETURN CAST([1.0] AS VECTOR) AS value",
        "RETURN CAST([1.0] AS VECTOR) IS TYPED VECTOR AS ok",
        "RETURN $value :: VECTOR AS value",
        "CREATE NODE TYPE :Embedding (embedding :: VECTOR)",
        "CREATE NODE TYPE :Embedding (embedding :: LIST<VECTOR>)",
        "CREATE NODE TYPE :Embedding (metadata :: RECORD{embedding :: VECTOR})",
    ] {
        parse(source)
            .unwrap_or_else(|err| panic!("VECTOR type did not parse for `{source}`: {err:?}"));
    }
}

#[test]
fn vector_remains_available_as_an_identifier() {
    parse("MATCH (vector) RETURN vector").expect("vector parses as an identifier");
}

#[test]
fn vector_type_name_records_implementation_defined_feature() {
    for source in [
        "RETURN CAST([1.0] AS VECTOR) AS value",
        "RETURN CAST([1.0] AS VECTOR) IS TYPED VECTOR AS ok",
        "CREATE NODE TYPE :Embedding (embedding :: VECTOR)",
    ] {
        let observed = feature_ids(source);
        assert!(
            observed.contains(&FeatureId::IM_VECTOR),
            "{source} should record IM_VECTOR, observed {observed:?}"
        );
    }
}

#[test]
fn cast_numeric_list_to_vector_returns_native_vector() {
    assert_eq!(
        first_value("RETURN CAST([1.0, 2.5, 3.0] AS VECTOR) AS value"),
        Value::Vector(VectorValue::new(vec![1.0, 2.5, 3.0]).expect("test vector is valid"))
    );
    assert_eq!(
        first_value("RETURN CAST([1.0] AS VECTOR) IS TYPED VECTOR AS value"),
        Value::Bool(true)
    );
}

#[test]
fn cast_empty_or_non_numeric_list_to_vector_is_datatype_mismatch() {
    for source in [
        "RETURN CAST([] AS VECTOR) AS value",
        "RETURN CAST(['bad'] AS VECTOR) AS value",
        "RETURN CAST([NULL] AS VECTOR) AS value",
    ] {
        assert_eq!(first_status(source), "22G03", "{source}");
    }
}

#[test]
fn cast_out_of_float32_range_component_returns_22003() {
    assert_eq!(
        bound_status(
            "RETURN CAST($value AS VECTOR) AS value",
            Value::List(vec![Value::Float(f64::MAX)])
        ),
        "22003"
    );
}

#[test]
fn non_list_cast_to_vector_is_datatype_mismatch() {
    let err = first_status("RETURN CAST(1 AS VECTOR) AS value");
    assert_eq!(err, GqlStatus::DATATYPE_MISMATCH.as_str());
}
