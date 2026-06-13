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
fn string_exact_numeric_casts_accept_decimal_literal_images() {
    assert_eq!(
        first_value("RETURN CAST('3.7' AS INTEGER) AS v"),
        Value::Int(3)
    );
    assert_eq!(
        first_value("RETURN CAST('-3.7' AS INT16) AS v"),
        Value::Int(-3)
    );
    assert_eq!(
        first_value("RETURN CAST('255.9' AS UINT8) AS v"),
        Value::Uint(255)
    );
    assert_eq!(
        first_value("RETURN CAST('3.7' AS INT128) AS v"),
        Value::Int128(3)
    );
}

#[test]
fn string_exact_numeric_casts_accept_scientific_and_suffix_images() {
    assert_eq!(
        first_value("RETURN CAST('2.0e2' AS INTEGER) AS v"),
        Value::Int(200)
    );
    assert_eq!(
        first_value("RETURN CAST('2.9d' AS INTEGER) AS v"),
        Value::Int(2)
    );
    assert_eq!(
        first_value("RETURN CAST('2M' AS INTEGER) AS v"),
        Value::Int(2)
    );
    assert_eq!(
        first_value("RETURN CAST('123M' AS DECIMAL) AS v"),
        Value::Decimal("123".parse().expect("valid decimal"))
    );
    assert_eq!(
        first_value("RETURN CAST('123M' AS FLOAT) AS v"),
        Value::Float(123.0)
    );
}

#[test]
fn string_radix_numeric_casts_accept_literal_images() {
    assert_eq!(
        first_value("RETURN CAST('0xCAFE' AS INTEGER) AS v"),
        Value::Int(51_966)
    );
    assert_eq!(
        first_value("RETURN CAST('-0x1A' AS INT16) AS v"),
        Value::Int(-26)
    );
    assert_eq!(
        first_value("RETURN CAST('+0x10' AS INTEGER) AS v"),
        Value::Int(16)
    );
    assert_eq!(
        first_value("RETURN CAST('0o777' AS UINT64) AS v"),
        Value::Uint(511)
    );
    assert_eq!(
        first_value("RETURN CAST('0b1010' AS UINT128) AS v"),
        Value::Uint128(10)
    );
    assert_eq!(
        first_value("RETURN CAST('0x10' AS DECIMAL) AS v"),
        Value::Decimal("16".parse().expect("valid decimal"))
    );
    assert_eq!(
        first_value("RETURN CAST('0b1010' AS FLOAT) AS v"),
        Value::Float(10.0)
    );
}

#[test]
fn string_radix_numeric_casts_accept_digit_separators() {
    assert_eq!(
        first_value("RETURN CAST('0x_1F' AS INTEGER) AS v"),
        Value::Int(31)
    );
    assert_eq!(
        first_value("RETURN CAST('0xCA_FE' AS INTEGER) AS v"),
        Value::Int(51_966)
    );
    assert_eq!(
        first_value("RETURN CAST('0b1010_0001' AS UINT16) AS v"),
        Value::Uint(161)
    );
}

#[test]
fn string_radix_numeric_casts_reject_malformed_images() {
    for source in [
        "RETURN CAST('0x' AS INTEGER) AS v",
        "RETURN CAST('0x_' AS INTEGER) AS v",
        "RETURN CAST('0x1__2' AS INTEGER) AS v",
        "RETURN CAST('0o8' AS INTEGER) AS v",
        "RETURN CAST('0b102' AS INTEGER) AS v",
        "RETURN CAST('+0x10' AS UINT64) AS v",
    ] {
        assert_eq!(first_status(source), "22018", "{source}");
    }
}

#[test]
fn string_radix_numeric_casts_report_overflow() {
    assert_eq!(
        first_status("RETURN CAST('0x8000000000000000' AS INTEGER) AS v"),
        "22003"
    );
    assert_eq!(
        first_status("RETURN CAST('0x1_0000_0000_0000_0000' AS UINT64) AS v"),
        "22003"
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
fn string_numeric_casts_reject_malformed_decimal_images() {
    for source in [
        "RETURN CAST('1.2.3' AS INTEGER) AS v",
        "RETURN CAST('1e' AS UINT8) AS v",
        "RETURN CAST('M' AS DECIMAL) AS v",
        "RETURN CAST('1M2' AS FLOAT) AS v",
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

#[test]
fn string_float_casts_reject_non_literal_special_tokens() {
    for source in [
        "RETURN CAST('NaN' AS FLOAT) AS v",
        "RETURN CAST('nan' AS FLOAT32) AS v",
        "RETURN CAST('inf' AS FLOAT) AS v",
        "RETURN CAST('+Infinity' AS FLOAT) AS v",
        "RETURN CAST('-Infinity' AS FLOAT32) AS v",
    ] {
        assert_eq!(first_status(source), "22018", "{source}");
    }
}

#[test]
fn string_float_casts_report_numeric_overflow() {
    assert_eq!(first_status("RETURN CAST('1e9999' AS FLOAT) AS v"), "22003");
    assert_eq!(
        first_status("RETURN CAST('1e9999' AS FLOAT32) AS v"),
        "22003"
    );
}
