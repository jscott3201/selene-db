//! String-image numeric CAST conformance cases.

use selene_core::{GraphId, Value};
use selene_gql::{EmptyProcedureRegistry, Session, StatementOutput};
use selene_graph::SharedGraph;

fn first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_800));
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
    let graph = SharedGraph::new(GraphId::new(13_801));
    let mut session = Session::new(&graph);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

#[test]
fn string_numeric_casts_accept_digit_separators() {
    assert_eq!(
        first_value("RETURN CAST('1_000' AS INTEGER) AS v"),
        Value::Int(1_000)
    );
    assert_eq!(
        first_value("RETURN CAST('-1_000' AS INT16) AS v"),
        Value::Int(-1_000)
    );
    assert_eq!(
        first_value("RETURN CAST('18_446_744_073_709_551_615' AS UINT64) AS v"),
        Value::Uint(u64::MAX)
    );
    assert_eq!(
        first_value("RETURN CAST('1_000.5' AS FLOAT) AS v"),
        Value::Float(1_000.5)
    );
    assert_eq!(
        first_value("RETURN CAST('1_000.5d' AS FLOAT) AS v"),
        Value::Float(1_000.5)
    );
    assert_eq!(
        first_value("RETURN CAST('1_000.5' AS FLOAT32) AS v"),
        Value::Float32(1_000.5_f32)
    );
    assert_eq!(
        first_value("RETURN CAST('1_000.50' AS DECIMAL) AS v"),
        Value::Decimal("1000.50".parse().expect("valid decimal"))
    );
}

#[test]
fn string_numeric_casts_reject_invalid_digit_separators() {
    for source in [
        "RETURN CAST('1__000' AS INTEGER) AS v",
        "RETURN CAST('_1000' AS INTEGER) AS v",
        "RETURN CAST('1000_' AS INTEGER) AS v",
        "RETURN CAST('+_1000' AS INTEGER) AS v",
        "RETURN CAST('1_.5' AS FLOAT) AS v",
        "RETURN CAST('1._5' AS DECIMAL) AS v",
    ] {
        assert_eq!(first_status(source), "22018", "{source}");
    }
}

#[test]
fn unsigned_string_numeric_casts_reject_explicit_signs() {
    for source in [
        "RETURN CAST('+1' AS UINT8) AS v",
        "RETURN CAST('-1' AS UINT8) AS v",
    ] {
        assert_eq!(first_status(source), "22018", "{source}");
    }
}

#[test]
fn string_numeric_cast_overflow_allows_digit_separators() {
    assert_eq!(
        first_status("RETURN CAST('9_223_372_036_854_775_808' AS INTEGER) AS v"),
        "22003"
    );
    assert_eq!(
        first_status(
            "RETURN CAST('340_282_366_920_938_463_463_374_607_431_768_211_456' AS UINT128) AS v"
        ),
        "22003"
    );
}
