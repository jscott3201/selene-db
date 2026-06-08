//! Duration scalar function evaluation (ISO/IEC 39075:2024 section 20.29).

use std::collections::BTreeSet;

use selene_core::{DbString, Record, Value};

use crate::{
    GqlType, SourceSpan,
    runtime::{DataExceptionSubclass, EvalCtx, ExecutorError},
    temporal_parse,
};

/// `DURATION(<string | record>)`: parse a duration string or build one from a
/// duration record constructor.
pub(super) fn eval_duration_function(
    args: Vec<Value>,
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let mut args = args;
    debug_assert_eq!(args.len(), 1);
    match args.pop().expect("arity checked by caller") {
        Value::Null => Ok(Value::Null),
        Value::String(text) => parse_duration_value(text.as_str().trim(), span),
        Value::Record(record) => match *record {
            Record::Open(fields) => duration_from_record(&fields.into_vec(), ctx, span),
            _ => Err(ExecutorError::data_exception(
                DataExceptionSubclass::InvalidValueType,
                "duration function argument is not an open record",
                span,
            )),
        },
        _ => Err(ExecutorError::data_exception(
            DataExceptionSubclass::InvalidValueType,
            "duration function argument is not a string or open record",
            span,
        )),
    }
}

fn duration_from_record(
    fields: &[(DbString, Value)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let family = duration_family(fields, span)?;
    let text = match family {
        DurationFamily::YearMonth => year_month_duration_text(fields, ctx, span)?,
        DurationFamily::DayTime => day_time_duration_text(fields, ctx, span)?,
    };
    parse_duration_value(&text, span)
}

fn parse_duration_value(text: &str, span: SourceSpan) -> Result<Value, ExecutorError> {
    temporal_parse::parse_duration(text)
        .map(|value| Value::Duration(Box::new(value)))
        .map_err(|error| invalid_duration_format(error, span))
}

#[derive(Clone, Copy)]
enum DurationFamily {
    YearMonth,
    DayTime,
}

fn duration_family(
    fields: &[(DbString, Value)],
    span: SourceSpan,
) -> Result<DurationFamily, ExecutorError> {
    let names = field_names(fields, span)?;
    let has_year_month = names.iter().any(|name| YEAR_MONTH_FIELDS.contains(name));
    let has_day_time = names.iter().any(|name| DAY_TIME_FIELDS.contains(name));
    let all_known = names
        .iter()
        .all(|name| YEAR_MONTH_FIELDS.contains(name) || DAY_TIME_FIELDS.contains(name));
    match (all_known, has_year_month, has_day_time) {
        (true, true, false) => Ok(DurationFamily::YearMonth),
        (true, false, true) => Ok(DurationFamily::DayTime),
        _ => Err(invalid_duration_field_name(
            "invalid DURATION constructor record fields",
            span,
        )),
    }
}

const YEAR_MONTH_FIELDS: &[&str] = &["years", "months"];
const DAY_TIME_FIELDS: &[&str] = &[
    "days",
    "hours",
    "minutes",
    "seconds",
    "milliseconds",
    "microseconds",
    "nanoseconds",
];
const SUBSECOND_FIELDS: &[&str] = &["milliseconds", "microseconds", "nanoseconds"];

fn year_month_duration_text(
    fields: &[(DbString, Value)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<String, ExecutorError> {
    let years = optional_field_text(fields, "years", "0", ctx, span)?;
    let months = optional_field_text(fields, "months", "0", ctx, span)?;
    Ok(format!("P{years}Y{months}M"))
}

fn day_time_duration_text(
    fields: &[(DbString, Value)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<String, ExecutorError> {
    validate_subsecond_fields(fields, span)?;
    let days = optional_field_text(fields, "days", "0", ctx, span)?;
    let hours = optional_field_text(fields, "hours", "0", ctx, span)?;
    let minutes = optional_field_text(fields, "minutes", "0", ctx, span)?;
    let seconds = optional_field_text(fields, "seconds", "0", ctx, span)?;
    let subsecond = subsecond_text(fields, ctx, span)?;
    Ok(format!("P{days}DT{hours}H{minutes}M{seconds}.{subsecond}S"))
}

fn validate_subsecond_fields(
    fields: &[(DbString, Value)],
    span: SourceSpan,
) -> Result<(), ExecutorError> {
    let count = fields
        .iter()
        .filter(|(name, _)| SUBSECOND_FIELDS.contains(&name.as_str()))
        .count();
    if count <= 1 {
        Ok(())
    } else {
        Err(invalid_duration_field_name(
            "multiple DURATION subsecond fields",
            span,
        ))
    }
}

fn subsecond_text(
    fields: &[(DbString, Value)],
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<String, ExecutorError> {
    for (name, width) in [("milliseconds", 3), ("microseconds", 6), ("nanoseconds", 9)] {
        if let Some(text) = maybe_field_text(fields, name, ctx, span)? {
            return Ok(pad_zeros(&text, width));
        }
    }
    Ok("000".to_owned())
}

fn optional_field_text(
    fields: &[(DbString, Value)],
    name: &str,
    default: &str,
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<String, ExecutorError> {
    maybe_field_text(fields, name, ctx, span).map(|text| text.unwrap_or_else(|| default.to_owned()))
}

fn maybe_field_text(
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
        Ok(Value::Null) => Err(invalid_duration_format(
            format!("{name} duration field value is NULL"),
            span,
        )),
        Ok(_) | Err(_) => Err(invalid_duration_format(
            format!("{name} duration field value cannot be converted to STRING"),
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
            return Err(invalid_duration_field_name(
                "duplicate DURATION constructor record field",
                span,
            ));
        }
    }
    Ok(names)
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

fn invalid_duration_field_name(message: impl Into<String>, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::InvalidDurationFunctionFieldName,
        message,
        span,
    )
}

fn invalid_duration_format(message: impl Into<String>, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(DataExceptionSubclass::InvalidDurationFormat, message, span)
}
