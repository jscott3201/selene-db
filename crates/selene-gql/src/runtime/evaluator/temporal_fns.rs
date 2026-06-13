//! Current-datetime scalar functions (ISO/IEC 39075:2024 section 20.27).
//!
//! These functions read the per-session time-zone displacement threaded into
//! [`TxContext`](crate::runtime::TxContext) (default UTC per Annex B ID048) and
//! produce temporal [`Value`]s anchored to the statement's request timestamp.
//! Each call within one statement uses the same captured instant; the session
//! time zone selects how the instant is presented (zoned forms) and which local
//! wall-clock components are returned (local forms).

use std::collections::BTreeSet;

use selene_core::{DbString, Record, Value};

use crate::{
    GqlType, SourceSpan,
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

/// `LOCAL_TIMESTAMP`: the current local (zoneless) datetime, with wall-clock
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
        ConstructorInput::Record(fields) => date_from_record(&fields, ctx, span),
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
        ConstructorInput::Record(fields) => zoned_time_from_record(&fields, ctx, span),
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
        ConstructorInput::Record(fields) => zoned_datetime_from_record(&fields, ctx, span),
    }
}

/// `LOCAL_TIME`: the current local (zoneless) time, with wall-clock components
/// taken in the session time zone (ISO section 20.27).
pub(super) fn eval_localtime(ctx: &EvalCtx<'_, '_, '_, '_>) -> Result<Value, ExecutorError> {
    Ok(Value::LocalTime(now_zoned(ctx).time()))
}

/// `TIME([string])` / `LOCAL_TIME([string])`: current local time with no
/// argument, or a local time parsed from a time string argument.
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
        ConstructorInput::Record(fields) => local_time_from_record(&fields, ctx, span),
    }
}

/// `DATETIME([string])` / `LOCAL_DATETIME([string])`: current local datetime
/// with no argument, or a local datetime parsed from a datetime string
/// argument.
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
        ConstructorInput::Record(fields) => local_datetime_from_record(&fields, ctx, span),
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
    Record(Vec<(DbString, Value)>),
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
        Value::Record(record) => match *record {
            Record::Open(fields) => Ok(ConstructorInput::Record(fields.into_vec())),
            _ => Err(ExecutorError::data_exception(
                DataExceptionSubclass::InvalidValueType,
                "datetime constructor argument is not an open record",
                span,
            )),
        },
        _ => Err(ExecutorError::data_exception(
            DataExceptionSubclass::InvalidValueType,
            "datetime constructor argument is not a string or open record",
            span,
        )),
    }
}

fn invalid_datetime_format(message: String, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(DataExceptionSubclass::InvalidDatetimeFormat, message, span)
}

fn date_from_record(
    fields: &[(DbString, Value)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    validate_date_fields(fields, span)?;
    let text = date_text(fields, ctx, span)?;
    temporal_parse::parse_date(&text)
        .map(Value::Date)
        .map_err(|error| invalid_datetime_value(error, span))
}

fn zoned_time_from_record(
    fields: &[(DbString, Value)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    validate_time_fields(fields, true, span)?;
    let text = time_text(fields, ctx, span)?;
    temporal_parse::parse_zoned_time(&text)
        .map(|value| Value::ZonedTime(Box::new(value)))
        .map_err(|error| invalid_datetime_value(error, span))
}

fn local_time_from_record(
    fields: &[(DbString, Value)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    validate_time_fields(fields, false, span)?;
    let text = time_text(fields, ctx, span)?;
    temporal_parse::parse_local_time(&text)
        .map(Value::LocalTime)
        .map_err(|error| invalid_datetime_value(error, span))
}

fn zoned_datetime_from_record(
    fields: &[(DbString, Value)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    validate_datetime_fields(fields, true, span)?;
    let text = format!(
        "{}T{}",
        date_text(fields, ctx, span)?,
        time_text(fields, ctx, span)?
    );
    temporal_parse::parse_zoned_datetime(&text)
        .map(|value| Value::ZonedDateTime(Box::new(value)))
        .map_err(|error| invalid_datetime_value(error, span))
}

fn local_datetime_from_record(
    fields: &[(DbString, Value)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    validate_datetime_fields(fields, false, span)?;
    let text = format!(
        "{}T{}",
        date_text(fields, ctx, span)?,
        time_text(fields, ctx, span)?
    );
    temporal_parse::parse_local_datetime(&text)
        .map(Value::LocalDateTime)
        .map_err(|error| invalid_datetime_value(error, span))
}

fn validate_date_fields(
    fields: &[(DbString, Value)],
    span: SourceSpan,
) -> Result<(), ExecutorError> {
    let names = field_names(fields, span)?;
    let valid = names == set(["year"])
        || names == set(["year", "month"])
        || names == set(["year", "month", "day"]);
    if valid {
        Ok(())
    } else {
        Err(invalid_datetime_field_name(
            "invalid DATE constructor record fields",
            span,
        ))
    }
}

fn validate_datetime_fields(
    fields: &[(DbString, Value)],
    allow_timezone: bool,
    span: SourceSpan,
) -> Result<(), ExecutorError> {
    let names = field_names(fields, span)?;
    let mut time_names = names.clone();
    for required in ["year", "month", "day"] {
        if !time_names.remove(required) {
            return Err(invalid_datetime_field_name(
                "invalid DATETIME constructor record fields",
                span,
            ));
        }
    }
    if time_names.is_empty() || !valid_time_names(&time_names, allow_timezone) {
        return Err(invalid_datetime_field_name(
            "invalid DATETIME constructor record fields",
            span,
        ));
    }
    Ok(())
}

fn validate_time_fields(
    fields: &[(DbString, Value)],
    allow_timezone: bool,
    span: SourceSpan,
) -> Result<(), ExecutorError> {
    let names = field_names(fields, span)?;
    if valid_time_names(&names, allow_timezone) {
        Ok(())
    } else {
        Err(invalid_datetime_field_name(
            "invalid TIME constructor record fields",
            span,
        ))
    }
}

fn valid_time_names(names: &BTreeSet<&str>, allow_timezone: bool) -> bool {
    let mut base = names.clone();
    let has_timezone = base.remove("timezone");
    if has_timezone && !allow_timezone {
        return false;
    }
    if base.is_empty() {
        return has_timezone && allow_timezone;
    }

    let subsecond_count = ["millisecond", "microsecond", "nanosecond"]
        .into_iter()
        .filter(|name| base.contains(name))
        .count();
    if subsecond_count > 1 {
        return false;
    }
    let allowed = [
        "hour",
        "minute",
        "second",
        "millisecond",
        "microsecond",
        "nanosecond",
    ];
    if !base.iter().all(|name| allowed.contains(name)) {
        return false;
    }

    base == set(["hour"])
        || base == set(["hour", "minute"])
        || base == set(["hour", "minute", "second"])
        || (subsecond_count == 1
            && base.contains("hour")
            && base.contains("minute")
            && base.contains("second")
            && base.len() == 4)
}

fn date_text(
    fields: &[(DbString, Value)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<String, ExecutorError> {
    let year = padded_field(fields, "year", 4, ctx, span)?;
    let month = padded_optional_field(fields, "month", "1", 2, ctx, span)?;
    let day = padded_optional_field(fields, "day", "1", 2, ctx, span)?;
    Ok(format!("{year}-{month}-{day}"))
}

fn time_text(
    fields: &[(DbString, Value)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<String, ExecutorError> {
    let hour = padded_field(fields, "hour", 2, ctx, span)?;
    let minute = padded_optional_field(fields, "minute", "0", 2, ctx, span)?;
    let second = padded_optional_field(fields, "second", "0", 2, ctx, span)?;
    let subsecond = subsecond_text(fields, ctx, span)?;
    let timezone = optional_field_text(fields, "timezone", ctx, span)?.unwrap_or_default();
    Ok(format!("{hour}:{minute}:{second}.{subsecond}{timezone}"))
}

fn subsecond_text(
    fields: &[(DbString, Value)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<String, ExecutorError> {
    for (name, width, max) in [
        ("millisecond", 3, 999_i128),
        ("microsecond", 6, 999_999_i128),
        ("nanosecond", 9, 999_999_999_i128),
    ] {
        if let Some(text) = optional_field_text(fields, name, ctx, span)? {
            let value = text.parse::<i128>().map_err(|_| {
                invalid_datetime_value(format!("invalid {name} datetime field value"), span)
            })?;
            if !(0..=max).contains(&value) {
                return Err(invalid_datetime_value(
                    format!("{name} datetime field value is out of range"),
                    span,
                ));
            }
            return Ok(pad_zeros(&text, width));
        }
    }
    Ok("000".to_owned())
}

fn padded_field(
    fields: &[(DbString, Value)],
    name: &str,
    width: usize,
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<String, ExecutorError> {
    let text = required_field_text(fields, name, ctx, span)?;
    Ok(pad_zeros(&text, width))
}

fn padded_optional_field(
    fields: &[(DbString, Value)],
    name: &str,
    default: &str,
    width: usize,
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<String, ExecutorError> {
    let text = optional_field_text(fields, name, ctx, span)?.unwrap_or_else(|| default.to_owned());
    Ok(pad_zeros(&text, width))
}

fn required_field_text(
    fields: &[(DbString, Value)],
    name: &str,
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<String, ExecutorError> {
    optional_field_text(fields, name, ctx, span)?
        .ok_or_else(|| invalid_datetime_value(format!("missing {name} datetime field value"), span))
}

fn optional_field_text(
    fields: &[(DbString, Value)],
    name: &str,
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<Option<String>, ExecutorError> {
    fields
        .iter()
        .find(|(field, _)| field.as_str() == name)
        .map(|(_, value)| field_value_text(value, name, ctx, span))
        .transpose()
}

fn field_value_text(
    value: &Value,
    name: &str,
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<String, ExecutorError> {
    match super::cast::eval_cast(value.clone(), &GqlType::String, span, ctx) {
        Ok(Value::String(text)) => Ok(text.as_str().to_owned()),
        Ok(Value::Null) => Err(invalid_datetime_value(
            format!("{name} datetime field value is NULL"),
            span,
        )),
        Ok(_) | Err(_) => Err(invalid_datetime_value(
            format!("{name} datetime field value cannot be converted to STRING"),
            span,
        )),
    }
}

fn field_names(
    fields: &[(DbString, Value)],
    span: SourceSpan,
) -> Result<BTreeSet<&str>, ExecutorError> {
    let mut names = BTreeSet::new();
    for (name, _) in fields {
        if !names.insert(name.as_str()) {
            return Err(invalid_datetime_field_name(
                "duplicate datetime constructor record field",
                span,
            ));
        }
    }
    Ok(names)
}

fn set<const N: usize>(items: [&'static str; N]) -> BTreeSet<&'static str> {
    items.into_iter().collect()
}

fn pad_zeros(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        return text.to_owned();
    }
    let mut padded = String::with_capacity(width);
    padded.extend(std::iter::repeat_n('0', width - len));
    padded.push_str(text);
    padded
}

fn invalid_datetime_field_name(message: impl Into<String>, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::InvalidDatetimeFunctionFieldName,
        message,
        span,
    )
}

fn invalid_datetime_value(message: impl Into<String>, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::InvalidDatetimeFunctionValue,
        message,
        span,
    )
}
