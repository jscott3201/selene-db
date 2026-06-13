//! Runtime conformance for width-specific approximate numeric casts.

use selene_core::{GraphId, Value, db_string};
use selene_gql::{EmptyProcedureRegistry, Session, StatementOutput};
use selene_graph::SharedGraph;

fn first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_720));
    let mut session = Session::new(&graph);
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    let StatementOutput::Rows(table) = output else {
        panic!("`{source}` produced non-row output");
    };
    table.rows()[0].values()[0].clone()
}

fn bound_status(source: &str, value: Value) -> String {
    let graph = SharedGraph::new(GraphId::new(13_721));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("p").expect("db_string param"), value);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

#[test]
fn cast_to_float32_returns_float32_value() {
    assert_eq!(
        first_value("RETURN CAST(1.5 AS FLOAT32) AS v"),
        Value::Float32(1.5_f32)
    );
    assert_eq!(
        first_value("RETURN CAST('1.5' AS FLOAT32) AS v"),
        Value::Float32(1.5_f32)
    );
    assert_eq!(
        first_value("RETURN CAST(1.5 AS FLOAT32) IS TYPED FLOAT32 AS ok"),
        Value::Bool(true)
    );
}

#[test]
fn floating_type_synonym_casts_use_concrete_widths() {
    assert_eq!(
        first_value("RETURN CAST(1.5 AS REAL) AS v"),
        Value::Float32(1.5_f32)
    );
    assert_eq!(
        first_value("RETURN CAST(1.5 AS DOUBLE) AS v"),
        Value::Float(1.5)
    );
    assert_eq!(
        first_value("RETURN CAST(1.5 AS DOUBLE PRECISION) IS TYPED DOUBLE AS ok"),
        Value::Bool(true)
    );
}

#[test]
fn cast_float32_overflow_returns_22003() {
    assert_eq!(
        bound_status("RETURN CAST($p AS FLOAT32) AS v", Value::Float(f64::MAX)),
        "22003"
    );
}
