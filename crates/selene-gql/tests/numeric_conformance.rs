//! 128-bit and DECIMAL numeric-completeness conformance tests.
//!
//! The GQL Flagger advertises GV13/GV14 (128-bit integers) and GV17 (DECIMAL)
//! as SUPPORTED. These end-to-end tests pin that the claim is honest across the
//! three reachable surfaces the deep-review ledger flagged (GQLRT-26 cross-type
//! equality, GQLRT-27 aggregation, GQLRT-30 arithmetic): `Int128`, `Uint128`,
//! and `Decimal` flow through `=`, `sum`/`avg`/`stddev`, and `+`/`-`/`*`
//! exactly like the 64-bit numerics, reaching the runtime via parameters.

use rust_decimal::Decimal;
use selene_core::{DbString, GraphId, Value};
use selene_gql::{EmptyProcedureRegistry, ExecutorError, Session, StatementOutput};
use selene_graph::SharedGraph;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn session_graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn execute(session: &mut Session<'_>, source: &str) -> Result<StatementOutput, ExecutorError> {
    session.execute_source(source, &EmptyProcedureRegistry)
}

fn single_value(session: &mut Session<'_>, source: &str) -> Value {
    let StatementOutput::Rows(table) = execute(session, source).expect("query succeeds") else {
        panic!("expected rows");
    };
    assert_eq!(table.row_count(), 1, "expected exactly one row");
    table.rows()[0].values()[0].clone()
}

// --- GQLRT-26: cross-type equality reachable through parameters ---

#[test]
fn int128_param_equals_int_literal() {
    let graph = session_graph(9001);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("i128"), Value::Int128(1));

    assert_eq!(
        single_value(&mut session, "RETURN $i128 = 1 AS eq"),
        Value::Bool(true)
    );
}

#[test]
fn uint128_param_equals_int_literal() {
    let graph = session_graph(9002);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("u128"), Value::Uint128(42));

    assert_eq!(
        single_value(&mut session, "RETURN $u128 = 42 AS eq"),
        Value::Bool(true)
    );
}

#[test]
fn decimal_param_equals_integer_literal() {
    let graph = session_graph(9003);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("dec"), Value::Decimal(Decimal::from(7)));

    assert_eq!(
        single_value(&mut session, "RETURN $dec = 7 AS eq"),
        Value::Bool(true)
    );
}

// --- GQLRT-27: aggregation over 128-bit / Decimal columns ---

#[test]
fn sum_over_int128_column() {
    let graph = session_graph(9010);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("a"), Value::Int128(10));
    session.bind_parameter(db_string("b"), Value::Int128(20));

    assert_eq!(
        single_value(&mut session, "UNWIND [$a, $b] AS x RETURN sum(x) AS s"),
        Value::Int128(30)
    );
}

#[test]
fn sum_over_uint128_column_out_of_i64_range() {
    let graph = session_graph(9011);
    let mut session = Session::new(&graph);
    // Each value exceeds i64::MAX, which the old `numeric_value` rejected.
    session.bind_parameter(db_string("a"), Value::Uint128(u128::from(u64::MAX)));
    session.bind_parameter(db_string("b"), Value::Uint128(1));

    assert_eq!(
        single_value(&mut session, "UNWIND [$a, $b] AS x RETURN sum(x) AS s"),
        Value::Int128(i128::from(u64::MAX) + 1)
    );
}

#[test]
fn sum_over_int128_column_overflow_is_numeric_out_of_range() {
    let graph = session_graph(9015);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("a"), Value::Int128(i128::MAX));
    session.bind_parameter(db_string("b"), Value::Int128(i128::MAX));

    let err = execute(&mut session, "UNWIND [$a, $b] AS x RETURN sum(x) AS s")
        .expect_err("i128 sum overflow errors");
    assert_eq!(err.gqlstatus().as_str(), "22003");
}

#[test]
fn sum_over_decimal_column() {
    let graph = session_graph(9012);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("a"), Value::Decimal("1.5".parse().unwrap()));
    session.bind_parameter(db_string("b"), Value::Decimal("2.5".parse().unwrap()));

    assert_eq!(
        single_value(&mut session, "UNWIND [$a, $b] AS x RETURN sum(x) AS s"),
        Value::Decimal("4.0".parse().unwrap())
    );
}

#[test]
fn avg_over_decimal_column() {
    let graph = session_graph(9013);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("a"), Value::Decimal("1".parse().unwrap()));
    session.bind_parameter(db_string("b"), Value::Decimal("2".parse().unwrap()));

    assert_eq!(
        single_value(&mut session, "UNWIND [$a, $b] AS x RETURN avg(x) AS a"),
        Value::Decimal("1.5".parse().unwrap())
    );
}

#[test]
fn stddev_pop_over_int128_column() {
    let graph = session_graph(9014);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("a"), Value::Int128(2));
    session.bind_parameter(db_string("b"), Value::Int128(4));

    // population stddev of [2,4] = 1.0
    assert_eq!(
        single_value(
            &mut session,
            "UNWIND [$a, $b] AS x RETURN stddev_pop(x) AS s"
        ),
        Value::Float(1.0)
    );
}

// --- GQLRT-30: arithmetic over Decimal / Uint128 ---

#[test]
fn decimal_param_addition() {
    let graph = session_graph(9020);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("a"), Value::Decimal("1.25".parse().unwrap()));
    session.bind_parameter(db_string("b"), Value::Decimal("2.75".parse().unwrap()));

    assert_eq!(
        single_value(&mut session, "RETURN $a + $b AS s"),
        Value::Decimal("4.00".parse().unwrap())
    );
}

#[test]
fn decimal_param_multiplication() {
    let graph = session_graph(9021);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("a"), Value::Decimal("1.5".parse().unwrap()));
    session.bind_parameter(db_string("b"), Value::Decimal("4".parse().unwrap()));

    assert_eq!(
        single_value(&mut session, "RETURN $a * $b AS s"),
        Value::Decimal("6.0".parse().unwrap())
    );
}

#[test]
fn uint128_param_addition() {
    let graph = session_graph(9022);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("a"), Value::Uint128(3));
    session.bind_parameter(db_string("b"), Value::Uint128(4));

    assert_eq!(
        single_value(&mut session, "RETURN $a + $b AS s"),
        Value::Uint128(7)
    );
}

#[test]
fn uint128_addition_overflow_is_numeric_out_of_range() {
    let graph = session_graph(9023);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("a"), Value::Uint128(u128::MAX));
    session.bind_parameter(db_string("b"), Value::Uint128(1));

    let err = execute(&mut session, "RETURN $a + $b AS s").expect_err("overflow errors");
    assert_eq!(err.gqlstatus().as_str(), "22003");
}

#[test]
fn decimal_division_by_zero_is_division_by_zero() {
    let graph = session_graph(9024);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("a"), Value::Decimal("1".parse().unwrap()));
    session.bind_parameter(db_string("b"), Value::Decimal("0".parse().unwrap()));

    let err = execute(&mut session, "RETURN $a / $b AS s").expect_err("division by zero errors");
    assert_eq!(err.gqlstatus().as_str(), "22012");
}
