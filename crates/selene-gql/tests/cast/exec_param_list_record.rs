//! CAST execution coverage — session-parameter sources (widened numerics,
//! DECIMAL), list element-wise casts, and record casts, split out of the
//! root `cast` binary to keep both files under the repository 700-LOC cap.

use selene_core::{GraphId, Record, Value, db_string};
use selene_gql::{EmptyProcedureRegistry, Session};
use selene_graph::SharedGraph;

use super::{
    as_string, execute_first_status, execute_first_value, execute_with_param,
    execute_with_param_status,
};

#[test]
fn cast_string_to_decimal_and_back_round_trips() {
    // String → DECIMAL (GR4g(ii)) then DECIMAL → STRING (GR4j canonical).
    let value = execute_first_value("RETURN CAST(CAST('123.45' AS DECIMAL) AS STRING) AS v");
    assert_eq!(as_string(value), "123.45");
}

#[test]
fn cast_string_parse_fail_to_decimal_returns_22018() {
    assert_eq!(
        execute_first_status("RETURN CAST('not-a-number' AS DECIMAL) AS v"),
        "22018"
    );
}

#[test]
fn cast_decimal_source_to_integer_truncates() {
    // DECIMAL → INTEGER: truncate toward zero.
    let value = execute_first_value("RETURN CAST(CAST('3.7' AS DECIMAL) AS INTEGER) AS v");
    assert_eq!(value, Value::Int(3));
}

#[test]
fn cast_decimal_source_to_float() {
    let value = execute_first_value("RETURN CAST(CAST('2.5' AS DECIMAL) AS FLOAT) AS v");
    assert_eq!(value, Value::Float(2.5));
}

#[test]
fn cast_int_literal_to_decimal_then_string() {
    // Int → DECIMAL (GR4g) end-to-end via nested CAST.
    let value = execute_first_value("RETURN CAST(CAST(42 AS DECIMAL) AS STRING) AS v");
    assert_eq!(as_string(value), "42");
}

#[test]
fn cast_uint_param_to_integer_widens() {
    assert_eq!(
        execute_with_param("RETURN CAST($p AS INTEGER) AS v", "p", Value::Uint(7)),
        Value::Int(7)
    );
}

#[test]
fn cast_uint_param_above_i64_max_to_integer_returns_22003() {
    assert_eq!(
        execute_with_param_status(
            "RETURN CAST($p AS INTEGER) AS v",
            "p",
            Value::Uint(u64::MAX)
        ),
        "22003"
    );
}

#[test]
fn cast_int128_param_to_float_widens() {
    assert_eq!(
        execute_with_param("RETURN CAST($p AS FLOAT) AS v", "p", Value::Int128(-9)),
        Value::Float(-9.0)
    );
}

#[test]
fn cast_uint128_param_to_string_widens() {
    assert_eq!(
        as_string(execute_with_param(
            "RETURN CAST($p AS STRING) AS v",
            "p",
            Value::Uint128(123)
        )),
        "123"
    );
}

#[test]
fn cast_float32_param_to_integer_truncates() {
    assert_eq!(
        execute_with_param(
            "RETURN CAST($p AS INTEGER) AS v",
            "p",
            Value::Float32(3.7_f32)
        ),
        Value::Int(3)
    );
}

#[test]
fn cast_float_special_values_to_string_render_symbolically() {
    // `format_float` renders the non-finite FLOAT values symbolically per the
    // numeric→C (GR4j) shortest-conforming-literal path: NaN → "NaN",
    // ±Infinity → "Infinity"/"-Infinity". These have no GQL literal, so they
    // are threaded through a bound FLOAT parameter (the only reachable surface
    // for the `format_float` special-case branches).
    assert_eq!(
        as_string(execute_with_param(
            "RETURN CAST($p AS STRING) AS v",
            "p",
            Value::Float(f64::NAN)
        )),
        "NaN"
    );
    assert_eq!(
        as_string(execute_with_param(
            "RETURN CAST($p AS STRING) AS v",
            "p",
            Value::Float(f64::INFINITY)
        )),
        "Infinity"
    );
    assert_eq!(
        as_string(execute_with_param(
            "RETURN CAST($p AS STRING) AS v",
            "p",
            Value::Float(f64::NEG_INFINITY)
        )),
        "-Infinity"
    );
}

#[test]
fn cast_decimal_param_to_boolean_returns_22g03() {
    // ISO §20.8 Table 4 marks EN→BO `N`; DECIMAL is signed-exact (EN).
    assert_eq!(
        execute_with_param_status(
            "RETURN CAST($p AS BOOLEAN) AS v",
            "p",
            Value::Decimal("1".parse().unwrap())
        ),
        "22G03"
    );
}

#[test]
fn cast_null_to_any_returns_null() {
    // §22 universal — NULL casts to NULL regardless of target.
    assert_eq!(
        execute_first_value("RETURN CAST(NULL AS INTEGER) AS v"),
        Value::Null
    );
    assert_eq!(
        execute_first_value("RETURN CAST(NULL AS STRING) AS v"),
        Value::Null
    );
    assert_eq!(
        execute_first_value("RETURN CAST(NULL AS BOOLEAN) AS v"),
        Value::Null
    );
    assert_eq!(
        execute_first_value("RETURN CAST(NULL AS LIST<INTEGER>) AS v"),
        Value::Null
    );
}

#[test]
fn cast_list_integer_to_string_element_wise() {
    // Q5 — LIST<T> casts element-wise. Source must also be a list.
    let value = execute_first_value("RETURN CAST([1, 2, 3] AS LIST<STRING>) AS v");
    let Value::List(items) = value else {
        panic!("expected list, got {value:?}");
    };
    assert_eq!(items.len(), 3);
    for (idx, item) in items.into_iter().enumerate() {
        assert_eq!(as_string(item), (idx + 1).to_string());
    }
}

#[test]
fn cast_bounded_list_enforces_max_cardinality_before_element_cast() {
    let value = execute_first_value("RETURN CAST([1, 2] AS LIST<STRING>[2]) AS v");
    let Value::List(items) = value else {
        panic!("expected list, got {value:?}");
    };
    assert_eq!(items.len(), 2);
    assert_eq!(as_string(items[0].clone()), "1");
    assert_eq!(as_string(items[1].clone()), "2");

    assert_eq!(
        execute_first_status("RETURN CAST([1, 2, 3] AS LIST<INTEGER>[2]) AS v"),
        "22G03"
    );
}

#[test]
fn cast_list_list_integer_to_list_list_string() {
    // Nested LIST recursion exercises the stacker-grown path in cast.rs.
    let value = execute_first_value("RETURN CAST([[1, 2], [3, 4]] AS LIST<LIST<STRING>>) AS v");
    let Value::List(outer) = value else {
        panic!("expected outer list, got {value:?}");
    };
    assert_eq!(outer.len(), 2);
    let mut flat = Vec::new();
    for inner in outer {
        let Value::List(items) = inner else {
            panic!("expected inner list, got {inner:?}");
        };
        for item in items {
            flat.push(as_string(item));
        }
    }
    assert_eq!(flat, vec!["1", "2", "3", "4"]);
}

#[test]
fn cast_node_to_anything_returns_42n01() {
    // Source NODE is rejected as 42N01 per ISO §22 (no defined image).
    // Seed the graph with one node so MATCH binds at least one row, then
    // attempt the cast in a second statement on the same session.
    let graph = SharedGraph::new(GraphId::new(13_510));
    let mut session = Session::new(&graph);
    session
        .execute_source("INSERT (:Person)", &EmptyProcedureRegistry)
        .expect("seed insert");
    let status = session
        .execute_source(
            "MATCH (n:Person) RETURN CAST(n AS STRING) AS v",
            &EmptyProcedureRegistry,
        )
        .expect_err("NODE cast rejects")
        .gqlstatus()
        .as_str()
        .to_owned();
    assert_eq!(status, "42N01");
}

#[test]
fn cast_path_to_anything_returns_42n01() {
    // Source PATH / EDGE is rejected per §A.3 (both NODE-family graph
    // elements are 42N01). Seed one edge and try to cast the edge
    // variable, which binds as `Value::EdgeRef` in the executor.
    let graph = SharedGraph::new(GraphId::new(13_511));
    let mut session = Session::new(&graph);
    session
        .execute_source("INSERT (:A)-[:REL]->(:B)", &EmptyProcedureRegistry)
        .expect("seed edge insert");
    let status = session
        .execute_source(
            "MATCH (a:A)-[r:REL]->(b:B) RETURN CAST(r AS STRING) AS v",
            &EmptyProcedureRegistry,
        )
        .expect_err("EDGE cast rejects")
        .gqlstatus()
        .as_str()
        .to_owned();
    assert_eq!(status, "42N01");
}

#[test]
fn cast_record_to_scalar_returns_22g03() {
    // A record source to a non-record (scalar) target is an invalid type combination per
    // ISO §20.8 Table 4 (`N`) → 22G03 datatype mismatch (not a missing feature).
    let source = "RETURN CAST({a: 1, b: 2} AS STRING) AS v";
    assert_eq!(execute_first_status(source), "22G03");
}

#[test]
fn cast_record_to_closed_record_coerces_and_projects() {
    // ISO §20.8 GR4(e): per-field recursive cast (string '5' -> INT 5); SR12 subset
    // projection drops the undeclared extra source field `b`.
    let source = "RETURN CAST({a: '5', b: 2} AS RECORD{a :: INT}) AS v";
    let value = execute_first_value(source);
    assert_eq!(
        value,
        Value::Record(Box::new(Record::Open(
            [(db_string("a").unwrap(), Value::Int(5))]
                .into_iter()
                .collect()
        )))
    );
}

#[test]
fn cast_record_missing_target_field_returns_22g0u() {
    // The target declares a field absent from the source → 22G0U record fields do not match.
    let source = "RETURN CAST({a: 1} AS RECORD{a :: INT, b :: STRING}) AS v";
    assert_eq!(execute_first_status(source), "22G0U");
}

#[test]
fn cast_non_record_to_record_returns_22g03() {
    let source = "RETURN CAST(5 AS RECORD{a :: INT}) AS v";
    assert_eq!(execute_first_status(source), "22G03");
}

#[test]
fn cast_record_to_open_record_is_identity() {
    let source = "RETURN CAST({a: 1, b: 'x'} AS RECORD) AS v";
    let value = execute_first_value(source);
    assert_eq!(
        value,
        Value::Record(Box::new(Record::Open(
            [
                (db_string("a").unwrap(), Value::Int(1)),
                (
                    db_string("b").unwrap(),
                    Value::String(db_string("x").unwrap())
                ),
            ]
            .into_iter()
            .collect()
        )))
    );
}

#[test]
fn cast_null_to_record_is_null() {
    let source = "RETURN CAST(NULL AS RECORD{a :: INT}) AS v";
    assert_eq!(execute_first_value(source), Value::Null);
}

#[test]
fn cast_record_inner_field_cast_failure_propagates() {
    // ISO §20.8 GR4(e)(i): each field is cast recursively, so a failing inner cast
    // ('abc' -> INT) must surface its own status (22018 invalid character value for cast),
    // not a swallowed or relabelled error.
    let source = "RETURN CAST({a: 'abc'} AS RECORD{a :: INT}) AS v";
    assert_eq!(execute_first_status(source), "22018");
}

#[test]
fn cast_record_to_nested_record_coerces_recursively() {
    // The recursive-descent path (cast_to_record re-enters under stacker::maybe_grow): a
    // nested closed-record target coerces the inner field ('5' -> INT 5).
    let source = "RETURN CAST({a: {b: '5'}} AS RECORD{a :: RECORD{b :: INT}}) AS v";
    let value = execute_first_value(source);
    assert_eq!(
        value,
        Value::Record(Box::new(Record::Open(
            [(
                db_string("a").unwrap(),
                Value::Record(Box::new(Record::Open(
                    [(db_string("b").unwrap(), Value::Int(5))]
                        .into_iter()
                        .collect()
                )))
            )]
            .into_iter()
            .collect()
        )))
    );
}

#[test]
fn cast_anything_to_null_returns_42n01() {
    // §A.3 — CAST to GqlType::Null is rejected as 42N01 (cannot cast TO
    // null). The grammar requires SQL-style explicit NULL as a type token;
    // selene-db's type grammar accepts `NULL` per `ast/types.rs:84`.
    let source = "RETURN CAST(1 AS NULL) AS v";
    assert_eq!(execute_first_status(source), "42N01");
}
