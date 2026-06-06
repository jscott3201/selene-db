//! Runtime conformance for width-specific numeric value types.

use selene_core::{GraphId, IStr, Value, intern};
use selene_gql::{EmptyProcedureRegistry, ExecutorError, Session, StatementOutput};
use selene_graph::SharedGraph;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
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
    session.bind_parameter(istr("p"), value);
    first_value_in(&mut session, source)
}

fn bind_and_error(value: Value, source: &str) -> ExecutorError {
    let graph = SharedGraph::new(GraphId::new(13_703));
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("p"), value);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
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
