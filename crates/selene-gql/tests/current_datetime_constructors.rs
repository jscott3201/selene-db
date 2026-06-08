//! Current-datetime constructor coverage for ISO/IEC 39075:2024 section 20.27.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::db_string;
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
        eval_current("localtimestamp"),
        Value::LocalDateTime(zoned.datetime())
    );
    assert_eq!(
        eval_current("local_datetime"),
        Value::LocalDateTime(zoned.datetime())
    );
    assert_eq!(eval_current("localtime"), Value::LocalTime(zoned.time()));
    assert_eq!(eval_current("local_time"), Value::LocalTime(zoned.time()));
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
        eval(&function_call("local_time", vec![string_lit("12:34:56")])).unwrap(),
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
