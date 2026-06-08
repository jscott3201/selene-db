//! Current-datetime constructor coverage for ISO/IEC 39075:2024 section 20.27.

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

fn lit(literal: Literal) -> ValueExpr {
    ValueExpr::Literal(literal)
}

fn string_lit(value: &str) -> ValueExpr {
    lit(Literal::String(db_string(value), span()))
}

fn null_lit() -> ValueExpr {
    lit(Literal::Null(span()))
}

fn bool_lit(value: bool) -> ValueExpr {
    lit(Literal::Bool(value, span()))
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
fn current_datetime_functions_share_one_request_timestamp() {
    let caps = selene_gql::ImplDefinedCaps::default();
    let ctx = exec_common::empty_graph_context(&caps);
    let schema = BindingTableSchema { columns: vec![] };
    let binding = Binding::empty();
    let eval_current = |name: &str| {
        selene_gql::runtime::evaluate_for_test(
            &function_call(name, vec![]),
            &binding,
            &schema,
            &ctx,
        )
        .expect("current-datetime function evaluates")
    };

    let current_timestamp = eval_current("current_timestamp");
    let Value::ZonedDateTime(zoned) = &current_timestamp else {
        panic!("current_timestamp produced {current_timestamp:?}");
    };
    std::thread::sleep(std::time::Duration::from_millis(2));

    assert_eq!(eval_current("current_timestamp"), current_timestamp);
    assert_eq!(eval_current("zoned_datetime"), current_timestamp);
    assert_eq!(
        eval_current("current_time"),
        Value::ZonedTime(zoned.clone())
    );
    assert_eq!(eval_current("zoned_time"), Value::ZonedTime(zoned.clone()));
    assert_eq!(eval_current("current_date"), Value::Date(zoned.date()));
    assert_eq!(eval_current("date"), Value::Date(zoned.date()));
    assert_eq!(
        eval_current("local_datetime"),
        Value::LocalDateTime(zoned.datetime())
    );
    assert_eq!(
        eval_current("datetime"),
        Value::LocalDateTime(zoned.datetime())
    );
    assert_eq!(eval_current("local_time"), Value::LocalTime(zoned.time()));
    assert_eq!(eval_current("time"), Value::LocalTime(zoned.time()));
}

#[test]
fn compact_local_datetime_aliases_are_not_scalar_functions() {
    assert_eq!(
        status_for("RETURN localtimestamp() AS value").as_str(),
        "22G03"
    );
    assert_eq!(status_for("RETURN localtime() AS value").as_str(), "22G03");
}

#[test]
fn current_datetime_constructors_parse_string_parameters() {
    assert_eq!(
        eval(&function_call("date", vec![string_lit("2026-05-07")])).unwrap(),
        Value::Date("2026-05-07".parse().unwrap())
    );

    let Value::ZonedTime(value) = eval(&function_call(
        "zoned_time",
        vec![string_lit("12:34:56-04:00")],
    ))
    .unwrap() else {
        panic!("zoned_time produced non-zoned-time value");
    };
    assert_eq!(value.time().to_string(), "12:34:56");
    assert_eq!(value.offset().to_string(), "-04");

    let Value::ZonedDateTime(value) = eval(&function_call(
        "zoned_datetime",
        vec![string_lit("2026-05-07T12:34:56-04:00")],
    ))
    .unwrap() else {
        panic!("zoned_datetime produced non-zoned-datetime value");
    };
    assert_eq!(value.datetime().to_string(), "2026-05-07T12:34:56");
    assert_eq!(value.offset().to_string(), "-04");

    assert_eq!(
        eval(&function_call(
            "local_datetime",
            vec![string_lit("2026-05-07T12:34:56")]
        ))
        .unwrap(),
        Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap())
    );
    assert_eq!(
        eval(&function_call(
            "datetime",
            vec![string_lit("2026-05-07T12:34:56")]
        ))
        .unwrap(),
        Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap())
    );
    assert_eq!(
        eval(&function_call("local_time", vec![string_lit("12:34:56")])).unwrap(),
        Value::LocalTime("12:34:56".parse().unwrap())
    );
    assert_eq!(
        eval(&function_call("time", vec![string_lit("12:34:56")])).unwrap(),
        Value::LocalTime("12:34:56".parse().unwrap())
    );

    assert_eq!(
        eval(&function_call("date", vec![null_lit()])).unwrap(),
        Value::Null
    );

    let err = eval(&function_call("date", vec![bool_lit(true)]))
        .expect_err("constructor rejects non-string parameters");
    assert_eq!(err.gqlstatus().as_str(), "22G03");

    let err = eval(&function_call("date", vec![string_lit("not-date")]))
        .expect_err("constructor rejects invalid temporal text");
    assert_eq!(err.gqlstatus(), GqlStatus::INVALID_DATETIME_FORMAT);
}

#[test]
fn current_datetime_record_constructors_build_values() {
    assert_eq!(
        single_value("RETURN DATE({year: 2026}) AS value", "value"),
        Value::Date("2026-01-01".parse().unwrap())
    );
    assert_eq!(
        single_value(
            "RETURN DATE(RECORD {year: 2026, month: 5, day: 7}) AS value",
            "value"
        ),
        Value::Date("2026-05-07".parse().unwrap())
    );
    assert_eq!(
        single_value(
            "RETURN LOCAL_TIME({hour: 1, minute: 2, second: 3, millisecond: 4}) AS value",
            "value"
        ),
        Value::LocalTime("01:02:03.004".parse().unwrap())
    );
    assert_eq!(
        single_value(
            "RETURN TIME({hour: 1, minute: 2, second: 3, millisecond: 4}) AS value",
            "value"
        ),
        Value::LocalTime("01:02:03.004".parse().unwrap())
    );
    assert_eq!(
        single_value(
            "RETURN LOCAL_DATETIME({year: 2026, month: 5, day: 7, hour: 12, minute: 34}) AS value",
            "value"
        ),
        Value::LocalDateTime("2026-05-07T12:34:00".parse().unwrap())
    );
    assert_eq!(
        single_value(
            "RETURN DATETIME({year: 2026, month: 5, day: 7, hour: 12, minute: 34}) AS value",
            "value"
        ),
        Value::LocalDateTime("2026-05-07T12:34:00".parse().unwrap())
    );

    let value = single_value(
        "RETURN ZONED_TIME({hour: 12, minute: 34, second: 56, timezone: '+03:00'}) AS value",
        "value",
    );
    let Value::ZonedTime(zoned_time) = value else {
        panic!("expected zoned time, got {value:?}");
    };
    assert_eq!(zoned_time.time().to_string(), "12:34:56");
    assert_eq!(zoned_time.offset().seconds(), 3 * 3600);

    let value = single_value(
        "RETURN ZONED_DATETIME({year: 2026, month: 5, day: 7, hour: 12, minute: 34, \
         second: 56, nanosecond: 7, timezone: '-04:00'}) AS value",
        "value",
    );
    let Value::ZonedDateTime(zoned_datetime) = value else {
        panic!("expected zoned datetime, got {value:?}");
    };
    assert_eq!(
        zoned_datetime.datetime().to_string(),
        "2026-05-07T12:34:56.000000007"
    );
    assert_eq!(zoned_datetime.offset().seconds(), -4 * 3600);
}

#[test]
fn current_datetime_record_constructors_reject_invalid_fields_and_values() {
    assert_eq!(
        status_for("RETURN DATE({month: 5}) AS value"),
        GqlStatus::INVALID_DATETIME_FUNCTION_FIELD_NAME
    );
    assert_eq!(
        status_for("RETURN LOCAL_TIME({hour: 1, timezone: '+00:00'}) AS value"),
        GqlStatus::INVALID_DATETIME_FUNCTION_FIELD_NAME
    );
    assert_eq!(
        status_for("RETURN LOCAL_DATETIME({year: 2026, month: 5, day: 7}) AS value"),
        GqlStatus::INVALID_DATETIME_FUNCTION_FIELD_NAME
    );

    assert_eq!(
        status_for("RETURN DATE({year: 2026, month: 13}) AS value"),
        GqlStatus::INVALID_DATETIME_FUNCTION_VALUE
    );
    assert_eq!(
        status_for(
            "RETURN LOCAL_TIME({hour: 1, minute: 2, second: 3, millisecond: 1000}) AS value"
        ),
        GqlStatus::INVALID_DATETIME_FUNCTION_VALUE
    );
    assert_eq!(
        status_for("RETURN ZONED_TIME({hour: 12}) AS value"),
        GqlStatus::INVALID_DATETIME_FUNCTION_VALUE
    );
    assert_eq!(
        status_for("RETURN ZONED_TIME({timezone: '+03:00'}) AS value"),
        GqlStatus::INVALID_DATETIME_FUNCTION_VALUE
    );
}
