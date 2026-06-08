//! Current-datetime scalar functions (ISO/IEC 39075:2024 section 20.27).
//!
//! These functions read the per-session time-zone displacement threaded into
//! [`TxContext`](crate::runtime::TxContext) (default UTC per Annex B ID048) and
//! produce temporal [`Value`]s anchored to the statement's request timestamp.
//! Each call within one statement uses the same captured instant; the session
//! time zone selects how the instant is presented (zoned forms) and which local
//! wall-clock components are returned (local forms).

use selene_core::Value;

use crate::{
    SourceSpan,
    runtime::{DataExceptionSubclass, EvalCtx, ExecutorError},
    temporal_parse,
};

/// `current_timestamp()`: the current zoned datetime in the session time zone
/// (ISO section 20.27).
pub(super) fn eval_current_timestamp(
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    Ok(Value::ZonedDateTime(Box::new(now_zoned(ctx))))
}

/// `localtimestamp()`: the current local (zoneless) datetime, with wall-clock
/// components taken in the session time zone (ISO section 20.27).
pub(super) fn eval_localtimestamp(ctx: &EvalCtx<'_, '_, '_, '_>) -> Result<Value, ExecutorError> {
    Ok(Value::LocalDateTime(now_zoned(ctx).datetime()))
}

/// `current_date()`: the current date in the session time zone (ISO section 20.27).
pub(super) fn eval_current_date(ctx: &EvalCtx<'_, '_, '_, '_>) -> Result<Value, ExecutorError> {
    Ok(Value::Date(now_zoned(ctx).date()))
}

/// `DATE([string])`: current date with no argument, or a date parsed from a
/// date string argument.
pub(super) fn eval_date_constructor(
    args: Vec<Value>,
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match constructor_input(args, span)? {
        ConstructorInput::Current => eval_current_date(ctx),
        ConstructorInput::Null => Ok(Value::Null),
        ConstructorInput::Text(text) => temporal_parse::parse_date(text.trim())
            .map(Value::Date)
            .map_err(|error| invalid_datetime_format(error, span)),
    }
}

/// `current_time()`: the current zoned time in the session time zone
/// (ISO section 20.27).
///
/// `jiff` 0.2 has no dedicated zoned-time type, so selene-core models a zoned
/// time as a [`jiff::Zoned`] (matching `Value::ZonedTime`).
pub(super) fn eval_current_time(ctx: &EvalCtx<'_, '_, '_, '_>) -> Result<Value, ExecutorError> {
    Ok(Value::ZonedTime(Box::new(now_zoned(ctx))))
}

/// `ZONED_TIME([string])`: current zoned time with no argument, or a zoned time
/// parsed from a time string argument.
pub(super) fn eval_zoned_time_constructor(
    args: Vec<Value>,
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match constructor_input(args, span)? {
        ConstructorInput::Current => eval_current_time(ctx),
        ConstructorInput::Null => Ok(Value::Null),
        ConstructorInput::Text(text) => temporal_parse::parse_zoned_time(text.trim())
            .map(|value| Value::ZonedTime(Box::new(value)))
            .map_err(|error| invalid_datetime_format(error, span)),
    }
}

/// `ZONED_DATETIME([string])`: current zoned datetime with no argument, or a
/// zoned datetime parsed from a datetime string argument.
pub(super) fn eval_zoned_datetime_constructor(
    args: Vec<Value>,
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match constructor_input(args, span)? {
        ConstructorInput::Current => eval_current_timestamp(ctx),
        ConstructorInput::Null => Ok(Value::Null),
        ConstructorInput::Text(text) => temporal_parse::parse_zoned_datetime(text.trim())
            .map(|value| Value::ZonedDateTime(Box::new(value)))
            .map_err(|error| invalid_datetime_format(error, span)),
    }
}

/// `localtime()`: the current local (zoneless) time, with wall-clock components
/// taken in the session time zone (ISO section 20.27).
pub(super) fn eval_localtime(ctx: &EvalCtx<'_, '_, '_, '_>) -> Result<Value, ExecutorError> {
    Ok(Value::LocalTime(now_zoned(ctx).time()))
}

/// `LOCAL_TIME([string])`: current local time with no argument, or a local time
/// parsed from a time string argument.
pub(super) fn eval_local_time_constructor(
    args: Vec<Value>,
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match constructor_input(args, span)? {
        ConstructorInput::Current => eval_localtime(ctx),
        ConstructorInput::Null => Ok(Value::Null),
        ConstructorInput::Text(text) => temporal_parse::parse_local_time(text.trim())
            .map(Value::LocalTime)
            .map_err(|error| invalid_datetime_format(error, span)),
    }
}

/// `LOCAL_DATETIME([string])`: current local datetime with no argument, or a
/// local datetime parsed from a datetime string argument.
pub(super) fn eval_local_datetime_constructor(
    args: Vec<Value>,
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match constructor_input(args, span)? {
        ConstructorInput::Current => eval_localtimestamp(ctx),
        ConstructorInput::Null => Ok(Value::Null),
        ConstructorInput::Text(text) => temporal_parse::parse_local_datetime(text.trim())
            .map(Value::LocalDateTime)
            .map_err(|error| invalid_datetime_format(error, span)),
    }
}

/// Capture the request timestamp rendered in the session time zone.
fn now_zoned(ctx: &EvalCtx<'_, '_, '_, '_>) -> jiff::Zoned {
    ctx.tx.request_timestamp_zoned()
}

enum ConstructorInput {
    Current,
    Null,
    Text(String),
}

fn constructor_input(
    mut args: Vec<Value>,
    span: SourceSpan,
) -> Result<ConstructorInput, ExecutorError> {
    let Some(value) = args.pop() else {
        return Ok(ConstructorInput::Current);
    };
    match value {
        Value::Null => Ok(ConstructorInput::Null),
        Value::String(value) => Ok(ConstructorInput::Text(value.as_str().to_owned())),
        _ => Err(ExecutorError::data_exception(
            DataExceptionSubclass::InvalidValueType,
            "datetime constructor argument is not a string",
            span,
        )),
    }
}

fn invalid_datetime_format(message: String, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(DataExceptionSubclass::InvalidDatetimeFormat, message, span)
}
