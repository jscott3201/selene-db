//! LIMIT/OFFSET parameter GQLSTATUS regressions.

use rust_decimal::Decimal;
use selene_core::{GraphId, Value, db_string};
use selene_gql::{
    DataExceptionSubclass, EmptyProcedureRegistry, ExecutorError, GqlStatus, Session,
    StatementOutput,
};
use selene_graph::SharedGraph;

fn empty_graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn row_count(session: &mut Session<'_>, source: &str) -> usize {
    match session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("query succeeds")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn limit_rows_with(value: Value, source: &str) -> usize {
    let graph = empty_graph(12_899);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("count").expect("test string fits"), value);
    row_count(&mut session, source)
}

#[test]
fn limit_parameter_accepts_wide_exact_integer_values() {
    let source = "FOR n IN [1, 2, 3] RETURN n LIMIT $count";

    assert_eq!(limit_rows_with(Value::Int128(2), source), 2);
    assert_eq!(limit_rows_with(Value::Uint128(2), source), 2);
    assert_eq!(limit_rows_with(Value::Decimal(Decimal::from(2)), source), 2);
}

#[test]
fn typed_limit_parameter_accepts_wide_exact_numeric_types() {
    assert_eq!(
        limit_rows_with(
            Value::Int128(2),
            "FOR n IN [1, 2, 3] RETURN n LIMIT $count :: INT128",
        ),
        2
    );
    assert_eq!(
        limit_rows_with(
            Value::Uint128(2),
            "FOR n IN [1, 2, 3] RETURN n LIMIT $count :: UINT128",
        ),
        2
    );
    assert_eq!(
        limit_rows_with(
            Value::Decimal(Decimal::from(2)),
            "FOR n IN [1, 2, 3] RETURN n LIMIT $count :: DECIMAL",
        ),
        2
    );
}

#[test]
fn offset_parameter_negative_reports_negative_limit_value() {
    let graph = empty_graph(12_900);
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("offset").expect("test string fits"),
        Value::Int(-1),
    );

    let err = session
        .execute_source(
            "MATCH (n:Sensor) RETURN n OFFSET $offset LIMIT 1",
            &EmptyProcedureRegistry,
        )
        .expect_err("negative offset is rejected");

    assert!(matches!(
        err,
        ExecutorError::DataException {
            subclass: DataExceptionSubclass::NegativeLimitValue,
            ..
        }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::NEGATIVE_LIMIT_VALUE);
}

#[test]
fn limit_parameter_null_reports_null_value_not_allowed() {
    let graph = empty_graph(12_901);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("count").expect("test string fits"), Value::Null);

    let err = session
        .execute_source(
            "MATCH (n:Sensor) RETURN n LIMIT $count",
            &EmptyProcedureRegistry,
        )
        .expect_err("null limit is rejected");

    assert!(matches!(
        err,
        ExecutorError::DataException {
            subclass: DataExceptionSubclass::NullValueNotAllowed,
            ..
        }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::NULL_VALUE_NOT_ALLOWED);
}

#[test]
fn decimal_limit_parameter_negative_integer_reports_negative_limit_value() {
    let graph = empty_graph(12_903);
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("count").expect("test string fits"),
        Value::Decimal(Decimal::from(-1)),
    );

    let err = session
        .execute_source(
            "FOR n IN [1, 2, 3] RETURN n LIMIT $count",
            &EmptyProcedureRegistry,
        )
        .expect_err("negative decimal limit is rejected");

    assert!(matches!(
        err,
        ExecutorError::DataException {
            subclass: DataExceptionSubclass::NegativeLimitValue,
            ..
        }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::NEGATIVE_LIMIT_VALUE);
}

#[test]
fn fractional_decimal_limit_parameter_reports_invalid_value_type() {
    for (id, value) in [(12_904, "1.5"), (12_905, "-1.5")] {
        let graph = empty_graph(id);
        let mut session = Session::new(&graph);
        session.bind_parameter(
            db_string("count").expect("test string fits"),
            Value::Decimal(value.parse().expect("decimal parses")),
        );

        let err = session
            .execute_source(
                "FOR n IN [1, 2, 3] RETURN n LIMIT $count",
                &EmptyProcedureRegistry,
            )
            .expect_err("fractional decimal limit is rejected");

        assert!(matches!(
            err,
            ExecutorError::DataException {
                subclass: DataExceptionSubclass::InvalidValueType,
                ..
            }
        ));
        assert_eq!(err.gqlstatus(), GqlStatus::DATATYPE_MISMATCH);
    }
}

#[test]
fn limit_parameter_above_u64_reports_numeric_value_out_of_range() {
    let graph = empty_graph(12_906);
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("count").expect("test string fits"),
        Value::Uint128(u128::from(u64::MAX) + 1),
    );

    let err = session
        .execute_source(
            "FOR n IN [1, 2, 3] RETURN n LIMIT $count",
            &EmptyProcedureRegistry,
        )
        .expect_err("out-of-range limit is rejected");

    assert!(matches!(
        err,
        ExecutorError::DataException {
            subclass: DataExceptionSubclass::NumericValueOutOfRange,
            ..
        }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::NUMERIC_VALUE_OUT_OF_RANGE);
}

#[test]
fn typed_limit_parameter_null_reports_null_value_not_allowed() {
    let graph = empty_graph(12_902);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("count").expect("test string fits"), Value::Null);

    let err = session
        .execute_source(
            "MATCH (n:Sensor) RETURN n LIMIT $count :: INT",
            &EmptyProcedureRegistry,
        )
        .expect_err("typed NULL LIMIT parameter is rejected by limit semantics");

    assert!(matches!(
        err,
        ExecutorError::DataException {
            subclass: DataExceptionSubclass::NullValueNotAllowed,
            ..
        }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::NULL_VALUE_NOT_ALLOWED);
}
