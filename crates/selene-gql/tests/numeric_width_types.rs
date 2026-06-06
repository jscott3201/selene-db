//! Runtime conformance for width-specific numeric value types.

use selene_core::{DbString, GraphId, Value};
use selene_gql::{EmptyProcedureRegistry, ExecutorError, Session, StatementOutput};
use selene_graph::SharedGraph;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_700));
    let mut session = Session::new(&graph);
    first_value_in(&mut session, source)
}

fn first_value_in(session: &mut Session<'_>, source: &str) -> Value {
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    let StatementOutput::Rows(table) = output else {
        panic!("`{source}` produced non-row output");
    };
    table.rows()[0].values()[0].clone()
}

fn first_status(source: &str) -> String {
    let graph = SharedGraph::new(GraphId::new(13_701));
    let mut session = Session::new(&graph);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

fn bind_and_eval(value: Value, source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_702));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("p"), value);
    first_value_in(&mut session, source)
}

fn bind_and_error(value: Value, source: &str) -> ExecutorError {
    let graph = SharedGraph::new(GraphId::new(13_703));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("p"), value);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
}

fn bind_and_status(value: Value, source: &str) -> String {
    bind_and_error(value, source)
        .gqlstatus()
        .as_str()
        .to_owned()
}

#[test]
fn is_typed_signed_integer_width_checks_range() {
    for (source, expected) in [
        ("RETURN 127 IS TYPED INT8 AS ok", true),
        ("RETURN 128 IS TYPED INT8 AS ok", false),
        ("RETURN -128 IS TYPED INT8 AS ok", true),
        ("RETURN -129 IS TYPED INT8 AS ok", false),
        ("RETURN 32767 IS TYPED SMALLINT AS ok", true),
        ("RETURN 32768 IS TYPED SMALLINT AS ok", false),
        ("RETURN 2147483647 IS TYPED INT32 AS ok", true),
        ("RETURN 2147483648 IS TYPED INT32 AS ok", false),
    ] {
        assert_eq!(first_value(source), Value::Bool(expected), "{source}");
    }
}

#[test]
fn is_typed_unsigned_integer_width_checks_range() {
    assert_eq!(
        bind_and_eval(Value::Uint(255), "RETURN $p IS TYPED UINT8 AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        bind_and_eval(Value::Uint(256), "RETURN $p IS TYPED UINT8 AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        bind_and_eval(
            Value::Uint(u64::from(u32::MAX)),
            "RETURN $p IS TYPED UINT32 AS ok"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        bind_and_eval(
            Value::Uint(u64::from(u32::MAX) + 1),
            "RETURN $p IS TYPED UINT32 AS ok"
        ),
        Value::Bool(false)
    );
}

#[test]
fn typed_parameters_enforce_numeric_width() {
    assert_eq!(
        bind_and_eval(Value::Int(127), "RETURN $p :: INT8 AS p"),
        Value::Int(127)
    );

    let err = bind_and_error(Value::Int(128), "RETURN $p :: INT8 AS p");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            ref expected,
            actual: "INTEGER",
            ..
        } if expected == "INT8"
    ));

    assert_eq!(
        bind_and_eval(Value::Uint(255), "RETURN $p :: UINT8 AS p"),
        Value::Uint(255)
    );
    let err = bind_and_error(Value::Uint(256), "RETURN $p :: UINT8 AS p");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            ref expected,
            actual: "UINT64",
            ..
        } if expected == "UINT8"
    ));
}

#[test]
fn typed_list_parameters_enforce_element_width() {
    assert_eq!(
        bind_and_eval(
            Value::List(vec![Value::Int(1), Value::Int(127)]),
            "RETURN $p :: LIST<INT8> AS p"
        ),
        Value::List(vec![Value::Int(1), Value::Int(127)])
    );

    let err = bind_and_error(
        Value::List(vec![Value::Int(1), Value::Int(128)]),
        "RETURN $p :: LIST<INT8> AS p",
    );
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            ref expected,
            actual: "LIST",
            ..
        } if expected == "LIST<INT8>"
    ));
}

#[test]
fn cast_to_signed_integer_width_checks_range() {
    assert_eq!(
        first_value("RETURN CAST(127 AS INT8) AS v"),
        Value::Int(127)
    );
    assert_eq!(
        first_value("RETURN CAST(2147483647 AS INT32) AS v"),
        Value::Int(i64::from(i32::MAX))
    );

    for source in [
        "RETURN CAST(128 AS INT8) AS v",
        "RETURN CAST(-129 AS INT8) AS v",
        "RETURN CAST(32768 AS SMALLINT) AS v",
        "RETURN CAST(2147483648 AS INT32) AS v",
    ] {
        assert_eq!(first_status(source), "22003", "{source}");
    }
}

#[test]
fn cast_string_to_signed_integer_checks_parse_and_range() {
    assert_eq!(
        first_value("RETURN CAST('9223372036854775807' AS INTEGER) AS v"),
        Value::Int(i64::MAX)
    );
    assert_eq!(
        first_value("RETURN CAST('-9223372036854775808' AS INTEGER) AS v"),
        Value::Int(i64::MIN)
    );

    for source in [
        "RETURN CAST('9223372036854775808' AS INTEGER) AS v",
        "RETURN CAST('-9223372036854775809' AS INTEGER) AS v",
        "RETURN CAST('9223372036854775808' AS INT8) AS v",
    ] {
        assert_eq!(first_status(source), "22003", "{source}");
    }
    assert_eq!(
        first_status("RETURN CAST('not-a-number' AS INTEGER) AS v"),
        "22018"
    );
}

#[test]
fn cast_to_int128_accepts_wide_numeric_sources() {
    assert_eq!(
        bind_and_eval(Value::Int128(i128::MIN), "RETURN CAST($p AS INT128) AS p"),
        Value::Int128(i128::MIN)
    );
    assert_eq!(
        bind_and_eval(
            Value::Uint128(i128::MAX as u128),
            "RETURN CAST($p AS INT128) AS p"
        ),
        Value::Int128(i128::MAX)
    );
    assert_eq!(
        bind_and_eval(Value::Float(3.7), "RETURN CAST($p AS INT128) AS p"),
        Value::Int128(3)
    );
    assert_eq!(
        bind_and_eval(
            Value::Decimal("-12.9".parse().expect("valid decimal")),
            "RETURN CAST($p AS INT128) AS p"
        ),
        Value::Int128(-12)
    );

    assert_eq!(
        bind_and_status(
            Value::Uint128((i128::MAX as u128) + 1),
            "RETURN CAST($p AS INT128) AS p"
        ),
        "22003"
    );
    assert_eq!(
        bind_and_status(Value::Float(f64::NAN), "RETURN CAST($p AS INT128) AS p"),
        "22018"
    );
}

#[test]
fn cast_string_to_int128_checks_parse_and_range() {
    assert_eq!(
        first_value("RETURN CAST('170141183460469231731687303715884105727' AS INT128) AS v"),
        Value::Int128(i128::MAX)
    );
    assert_eq!(
        first_value("RETURN CAST('-170141183460469231731687303715884105728' AS INT128) AS v"),
        Value::Int128(i128::MIN)
    );

    for source in [
        "RETURN CAST('170141183460469231731687303715884105728' AS INT128) AS v",
        "RETURN CAST('-170141183460469231731687303715884105729' AS INT128) AS v",
    ] {
        assert_eq!(first_status(source), "22003", "{source}");
    }
    assert_eq!(
        first_status("RETURN CAST('not-a-number' AS INT128) AS v"),
        "22018"
    );
}

#[test]
fn cast_to_unsigned_integer_width_checks_range() {
    assert_eq!(
        first_value("RETURN CAST(255 AS UINT8) AS v"),
        Value::Uint(255)
    );
    assert_eq!(
        first_value("RETURN CAST(4294967295 AS UINT32) AS v"),
        Value::Uint(u64::from(u32::MAX))
    );

    for source in [
        "RETURN CAST(256 AS UINT8) AS v",
        "RETURN CAST(-1 AS UINT8) AS v",
        "RETURN CAST(4294967296 AS UINT32) AS v",
    ] {
        assert_eq!(first_status(source), "22003", "{source}");
    }
}

#[test]
fn cast_string_to_unsigned_integer_checks_parse_and_range() {
    assert_eq!(
        first_value("RETURN CAST('255' AS UINT8) AS v"),
        Value::Uint(255)
    );
    assert_eq!(
        first_value("RETURN CAST(' 18446744073709551615 ' AS UINT64) AS v"),
        Value::Uint(u64::MAX)
    );
    assert_eq!(
        first_value("RETURN CAST('340282366920938463463374607431768211455' AS UINT128) AS v"),
        Value::Uint128(u128::MAX)
    );

    for source in [
        "RETURN CAST('-1' AS UINT8) AS v",
        "RETURN CAST('not-a-number' AS UINT8) AS v",
    ] {
        assert_eq!(first_status(source), "22018", "{source}");
    }
    assert_eq!(first_status("RETURN CAST('256' AS UINT8) AS v"), "22003");
    assert_eq!(
        first_status("RETURN CAST('340282366920938463463374607431768211456' AS UINT128) AS v"),
        "22003"
    );
}

#[test]
fn cast_bound_numeric_sources_to_unsigned_integer() {
    assert_eq!(
        bind_and_eval(Value::Uint128(u128::MAX), "RETURN CAST($p AS UINT128) AS p"),
        Value::Uint128(u128::MAX)
    );
    assert_eq!(
        bind_and_eval(Value::Float(3.7), "RETURN CAST($p AS UINT8) AS p"),
        Value::Uint(3)
    );
    assert_eq!(
        bind_and_eval(
            Value::Decimal("12.9".parse().expect("valid decimal")),
            "RETURN CAST($p AS UINT8) AS p"
        ),
        Value::Uint(12)
    );

    assert_eq!(
        bind_and_status(Value::Int(-1), "RETURN CAST($p AS UINT8) AS p"),
        "22003"
    );
    assert_eq!(
        bind_and_status(Value::Float(-0.1), "RETURN CAST($p AS UINT8) AS p"),
        "22003"
    );
    assert_eq!(
        bind_and_status(Value::Float(f64::NAN), "RETURN CAST($p AS UINT8) AS p"),
        "22018"
    );
    assert_eq!(
        bind_and_status(
            Value::Decimal("-1".parse().expect("valid decimal")),
            "RETURN CAST($p AS UINT8) AS p"
        ),
        "22003"
    );
}

#[test]
fn cast_boolean_to_unsigned_integer_returns_22g03() {
    assert_eq!(first_status("RETURN CAST(true AS UINT8) AS v"), "22G03");
    assert_eq!(first_status("RETURN CAST(true AS INT128) AS v"), "22G03");
}
