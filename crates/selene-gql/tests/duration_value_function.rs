//! Duration value function coverage for ISO/IEC 39075:2024 section 20.29.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{column_values, db_string, execute_read, execute_read_result};
use selene_core::Value;
use selene_gql::{
    Binding, BindingTableSchema, GqlStatus, Literal, NonEmpty, SourceSpan, ValueExpr,
};

fn span() -> SourceSpan {
    SourceSpan::new(0, 1)
}

fn string_lit(value: &str) -> ValueExpr {
    ValueExpr::Literal(Literal::String(db_string(value), span()))
}

fn null_lit() -> ValueExpr {
    ValueExpr::Literal(Literal::Null(span()))
}

fn bool_lit(value: bool) -> ValueExpr {
    ValueExpr::Literal(Literal::Bool(value, span()))
}

fn function_call(name: &str, args: Vec<ValueExpr>) -> ValueExpr {
    ValueExpr::FunctionCall {
        name: NonEmpty::try_from_vec(vec![db_string(name)]).expect("non-empty"),
        args,
        star: false,
        distinct: false,
        span: span(),
    }
}

fn eval(expr: &ValueExpr) -> Result<Value, selene_gql::ExecutorError> {
    let caps = selene_gql::ImplDefinedCaps::default();
    let ctx = exec_common::empty_graph_context(&caps);
    selene_gql::runtime::evaluate_for_test(
        expr,
        &Binding::empty(),
        &BindingTableSchema { columns: vec![] },
        &ctx,
    )
}

fn single_value(source: &str, column: &str) -> Value {
    let table = execute_read(source);
    let mut values = column_values(&table, column);
    assert_eq!(values.len(), 1, "{source}");
    values.pop().expect("one row")
}

fn status_for(source: &str) -> GqlStatus {
    execute_read_result(source)
        .expect_err("statement errors")
        .gqlstatus()
}

#[test]
fn duration_function_parses_string_and_null_parameters() {
    assert_eq!(
        eval(&function_call("duration", vec![string_lit("P2M")])).unwrap(),
        Value::Duration(Box::new("P2M".parse().unwrap()))
    );
    assert_eq!(
        eval(&function_call("duration", vec![null_lit()])).unwrap(),
        Value::Null
    );

    let err = eval(&function_call("duration", vec![bool_lit(true)]))
        .expect_err("duration rejects non-string/non-record parameters");
    assert_eq!(err.gqlstatus().as_str(), "22G03");

    let err = eval(&function_call("duration", vec![string_lit("not-duration")]))
        .expect_err("duration rejects invalid duration text");
    assert_eq!(err.gqlstatus(), GqlStatus::INVALID_DURATION_FORMAT);
}

#[test]
fn duration_record_constructor_builds_year_month_and_day_time_values() {
    assert_eq!(
        single_value("RETURN DURATION({years: 1, months: 2}) AS value", "value"),
        Value::Duration(Box::new("P1Y2M".parse().unwrap()))
    );
    assert_eq!(
        single_value("RETURN DURATION({months: 3}) AS value", "value"),
        Value::Duration(Box::new("P0Y3M".parse().unwrap()))
    );
    assert_eq!(
        single_value(
            "RETURN DURATION({days: 3, hours: 4, minutes: 5, seconds: 6, milliseconds: 7}) AS value",
            "value"
        ),
        Value::Duration(Box::new("P3DT4H5M6.007S".parse().unwrap()))
    );
    assert_eq!(
        single_value(
            "RETURN DURATION({days: 0, seconds: 1, nanoseconds: 2}) AS value",
            "value"
        ),
        Value::Duration(Box::new("P0DT0H0M1.000000002S".parse().unwrap()))
    );
}

#[test]
fn duration_record_constructor_rejects_invalid_fields_and_format_values() {
    assert_eq!(
        status_for("RETURN DURATION({foo: 1}) AS value"),
        GqlStatus::INVALID_DURATION_FUNCTION_FIELD_NAME
    );
    assert_eq!(
        status_for("RETURN DURATION({year: 1}) AS value"),
        GqlStatus::INVALID_DURATION_FUNCTION_FIELD_NAME
    );
    assert_eq!(
        status_for("RETURN DURATION({years: 1, days: 2}) AS value"),
        GqlStatus::INVALID_DURATION_FUNCTION_FIELD_NAME
    );
    assert_eq!(
        status_for("RETURN DURATION({seconds: 1, milliseconds: 2, nanoseconds: 3}) AS value"),
        GqlStatus::INVALID_DURATION_FUNCTION_FIELD_NAME
    );
    assert_eq!(
        status_for("RETURN DURATION({seconds: 'not-a-number'}) AS value"),
        GqlStatus::INVALID_DURATION_FORMAT
    );
}
