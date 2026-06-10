//! CAST execution coverage — scalar, numeric, boolean, and string arms,
//! split out of the root `cast` binary to keep both files under the
//! repository 700-LOC cap. Shares the root's execute helpers via `super::`.

use selene_core::{GraphId, Value};
use selene_gql::{EmptyProcedureRegistry, Session};
use selene_graph::SharedGraph;

use super::{as_string, execute_first_status, execute_first_value};

#[test]
fn cast_integer_to_string_round_trip() {
    assert_eq!(
        as_string(execute_first_value("RETURN CAST(42 AS STRING) AS v")),
        "42"
    );
}

#[test]
fn cast_string_to_integer_valid_parse() {
    assert_eq!(
        execute_first_value("RETURN CAST('42' AS INTEGER) AS v"),
        Value::Int(42)
    );
}

#[test]
fn cast_string_to_integer_returns_22018() {
    assert_eq!(
        execute_first_status("RETURN CAST('abc' AS INTEGER) AS v"),
        "22018"
    );
}

#[test]
fn cast_float_to_integer_truncates_toward_zero() {
    // ISO §22.4 — truncate toward zero. 3.7 -> 3, -3.7 -> -3.
    assert_eq!(
        execute_first_value("RETURN CAST(3.7D AS INTEGER) AS v"),
        Value::Int(3)
    );
    assert_eq!(
        execute_first_value("RETURN CAST(-3.7D AS INTEGER) AS v"),
        Value::Int(-3)
    );
}

#[test]
fn cast_float_to_integer_overflow_returns_22003() {
    // 1.0e30 is far beyond i64::MAX (~9.2e18); the explicit range check
    // fires before Rust's saturating `as` cast hides the overflow. GQL
    // approximate-literal grammar uses the `D` suffix because bare common
    // decimals are exact DECIMAL literals.
    assert_eq!(
        execute_first_status("RETURN CAST(1.0e30D AS INTEGER) AS v"),
        "22003"
    );
}

#[test]
fn cast_float_nan_to_integer_returns_22018() {
    // CAST of NaN to INTEGER has no representable image; ISO §22 emits
    // 22018 (invalid-character-value-for-cast). NaN has no GQL literal,
    // so the integration test threads `f64::NAN` through the runtime via
    // a session parameter binding — exercising the full parse → analyze
    // → plan → execute → evaluator pipeline end-to-end. The inline unit
    // test in `runtime/evaluator/cast.rs::tests::float_nan_to_integer_returns_22018`
    // pins the same branch in isolation.
    let graph = SharedGraph::new(GraphId::new(13_520));
    let mut session = Session::new(&graph);
    session.bind_parameter(
        selene_core::db_string("nan").expect("db_string parameter name"),
        Value::Float(f64::NAN),
    );
    let status = session
        .execute_source("RETURN CAST($nan AS INTEGER) AS v", &EmptyProcedureRegistry)
        .expect_err("NaN cast must reject at runtime")
        .gqlstatus()
        .as_str()
        .to_owned();
    assert_eq!(status, "22018");
}

#[test]
fn cast_integer_to_float_preserves_value() {
    assert_eq!(
        execute_first_value("RETURN CAST(42 AS FLOAT) AS v"),
        Value::Float(42.0)
    );
}

#[test]
fn cast_float_to_string_round_trip() {
    assert_eq!(
        as_string(execute_first_value("RETURN CAST(3.5D AS STRING) AS v")),
        "3.5"
    );
}

#[test]
fn cast_string_to_float_valid_parse() {
    assert_eq!(
        execute_first_value("RETURN CAST('2.5' AS FLOAT) AS v"),
        Value::Float(2.5)
    );
}

#[test]
fn cast_string_to_float_returns_22018() {
    assert_eq!(
        execute_first_status("RETURN CAST('2.5abc' AS FLOAT) AS v"),
        "22018"
    );
}

#[test]
fn cast_boolean_to_string_uppercase() {
    // ISO §20.8 GR4(j)(v)(1) / GR4v — boolean→string yields the UPPERCASE
    // literal "TRUE"/"FALSE" (810 strict-ISO fix; was lowercase).
    assert_eq!(
        as_string(execute_first_value("RETURN CAST(true AS STRING) AS v")),
        "TRUE"
    );
    assert_eq!(
        as_string(execute_first_value("RETURN CAST(false AS STRING) AS v")),
        "FALSE"
    );
}

#[test]
fn cast_string_to_boolean_case_insensitive() {
    // ISO §20.8 GR4(q) defers C→BO to the §21.2 boolean literal, which is
    // case-insensitive (810 strict-ISO fix; was strict-lowercase). Leading /
    // trailing whitespace is trimmed per GR4(g)(ii).
    for good in ["true", "True", "TRUE", "tRuE", "  true  "] {
        let source = format!("RETURN CAST('{good}' AS BOOLEAN) AS v");
        assert_eq!(
            execute_first_value(&source),
            Value::Bool(true),
            "input `{good}` must parse to TRUE"
        );
    }
    for good in ["false", "False", "FALSE", "fAlSe", " FALSE "] {
        let source = format!("RETURN CAST('{good}' AS BOOLEAN) AS v");
        assert_eq!(
            execute_first_value(&source),
            Value::Bool(false),
            "input `{good}` must parse to FALSE"
        );
    }
    // Non-boolean text still rejects with 22018.
    for bad in ["yes", "1", "t"] {
        let source = format!("RETURN CAST('{bad}' AS BOOLEAN) AS v");
        assert_eq!(
            execute_first_status(&source),
            "22018",
            "input `{bad}` must reject as 22018"
        );
    }
}

#[test]
fn cast_boolean_to_integer_returns_22g03() {
    // ISO §20.8 Table 4 marks BO→EN `N` — there is no boolean→numeric cast
    // (810 strict-ISO fix; the old 0/1 extension is removed). 22G03 datatype
    // mismatch.
    assert_eq!(
        execute_first_status("RETURN CAST(true AS INTEGER) AS v"),
        "22G03"
    );
    assert_eq!(
        execute_first_status("RETURN CAST(false AS INTEGER) AS v"),
        "22G03"
    );
}

#[test]
fn cast_boolean_to_float_returns_22g03() {
    // ISO §20.8 Table 4 marks BO→AN `N` (810 strict-ISO fix).
    assert_eq!(
        execute_first_status("RETURN CAST(true AS FLOAT) AS v"),
        "22G03"
    );
}

#[test]
fn cast_integer_to_boolean_returns_22g03() {
    // ISO §20.8 Table 4 marks EN→BO `N` — there is no numeric→boolean cast
    // (810 strict-ISO fix; the old 0/1 extension is removed). Every integer,
    // including 0/1, is now a 22G03 datatype mismatch.
    assert_eq!(
        execute_first_status("RETURN CAST(0 AS BOOLEAN) AS v"),
        "22G03"
    );
    assert_eq!(
        execute_first_status("RETURN CAST(1 AS BOOLEAN) AS v"),
        "22G03"
    );
    assert_eq!(
        execute_first_status("RETURN CAST(2 AS BOOLEAN) AS v"),
        "22G03"
    );
}

#[test]
fn cast_float_to_boolean_returns_22g03() {
    // ISO §20.8 Table 4 marks AN→BO `N` (810 strict-ISO fix).
    assert_eq!(
        execute_first_status("RETURN CAST(1.0D AS BOOLEAN) AS v"),
        "22G03"
    );
}

#[test]
fn cast_boolean_to_decimal_returns_22g03() {
    // ISO §20.8 Table 4 marks BO→EN `N`; DECIMAL is signed-exact (EN), so a
    // boolean→DECIMAL cast is a 22G03 datatype mismatch, not a 42N01
    // unimplemented feature — the DECIMAL target's source fallthrough must
    // classify BOOLEAN identically to the INTEGER/FLOAT targets (Codex P2 on
    // PR #240).
    assert_eq!(
        execute_first_status("RETURN CAST(true AS DECIMAL) AS v"),
        "22G03"
    );
}
