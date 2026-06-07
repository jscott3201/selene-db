//! Runtime status coverage for ISO §20.8 Table-4 invalid CAST cells.

use selene_core::{GraphId, Value, db_string};
use selene_gql::{EmptyProcedureRegistry, Session};
use selene_graph::SharedGraph;

fn first_status(source: &str) -> String {
    let graph = SharedGraph::new(GraphId::new(13_850));
    let mut session = Session::new(&graph);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement should reject")
        .gqlstatus()
        .as_str()
        .to_owned()
}

fn bound_status(source: &str, value: Value) -> String {
    let graph = SharedGraph::new(GraphId::new(13_851));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("p").expect("parameter name"), value);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement should reject")
        .gqlstatus()
        .as_str()
        .to_owned()
}

#[test]
fn non_list_source_to_list_target_returns_22g03() {
    for source in [
        "RETURN CAST(1 AS LIST<INTEGER>) AS value",
        "RETURN CAST('x' AS LIST<STRING>) AS value",
        "RETURN CAST(DATE '2026-05-07' AS LIST<DATE>) AS value",
    ] {
        assert_eq!(first_status(source), "22G03", "{source}");
    }
}

#[test]
fn list_source_to_scalar_target_returns_22g03() {
    for source in [
        "RETURN CAST([1] AS STRING) AS value",
        "RETURN CAST([1] AS BOOLEAN) AS value",
        "RETURN CAST([1] AS INTEGER) AS value",
        "RETURN CAST([1] AS DATE) AS value",
    ] {
        assert_eq!(first_status(source), "22G03", "{source}");
    }
}

#[test]
fn temporal_source_to_scalar_target_returns_22g03() {
    for source in [
        "RETURN CAST(DATE '2026-05-07' AS INTEGER) AS value",
        "RETURN CAST(LOCAL TIME '12:34:56' AS BOOLEAN) AS value",
        "RETURN CAST(DURATION 'PT1H' AS DECIMAL) AS value",
    ] {
        assert_eq!(first_status(source), "22G03", "{source}");
    }
}

#[test]
fn scalar_source_to_temporal_target_returns_22g03() {
    for source in [
        "RETURN CAST(1 AS DATE) AS value",
        "RETURN CAST(true AS LOCAL TIME) AS value",
        "RETURN CAST(1.5 AS DURATION) AS value",
    ] {
        assert_eq!(first_status(source), "22G03", "{source}");
    }
}

#[test]
fn dynamic_static_type_mismatches_return_22g03() {
    let date = Value::Date("2026-05-07".parse().expect("date"));
    assert_eq!(
        bound_status("RETURN CAST($p AS INTEGER) AS value", date),
        "22G03"
    );

    assert_eq!(
        bound_status("RETURN CAST($p AS DATE) AS value", Value::Int(1)),
        "22G03"
    );

    let time = Value::LocalTime("12:34:56".parse().expect("local time"));
    assert_eq!(
        bound_status("RETURN CAST($p AS BOOLEAN) AS value", time),
        "22G03"
    );

    assert_eq!(
        bound_status(
            "RETURN CAST($p AS STRING) AS value",
            Value::List(vec![Value::Int(1)])
        ),
        "22G03"
    );
}
