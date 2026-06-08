use jiff::SignedDuration;
use selene_core::Value;

use crate::{
    GqlType, SourceSpan,
    runtime::{DataExceptionSubclass, EvalCtx, ExecutorError},
    temporal_parse,
};

use super::non_iso_static_source_for_target;

pub(super) fn cast_to_temporal(
    value: Value,
    target_type: &GqlType,
    span: SourceSpan,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    match (value, target_type) {
        (Value::ZonedDateTime(value), GqlType::ZonedDateTime) => Ok(Value::ZonedDateTime(value)),
        (Value::ZonedDateTime(value), GqlType::LocalDateTime) => {
            shifted_zoned_datetime_local(&value, span).map(Value::LocalDateTime)
        }
        (Value::ZonedDateTime(value), GqlType::Date) => Ok(Value::Date(value.datetime().date())),
        (Value::ZonedDateTime(value), GqlType::LocalTime) => {
            shifted_zoned_datetime_local(&value, span)
                .map(|datetime| Value::LocalTime(datetime.time()))
        }
        (Value::ZonedDateTime(value), GqlType::ZonedTime) => {
            let text = format!("{}{}", value.time(), value.offset());
            temporal_parse::parse_zoned_time(&text)
                .map(|value| Value::ZonedTime(Box::new(value)))
                .map_err(|error| invalid_datetime_format(error, span))
        }
        (Value::LocalDateTime(value), GqlType::LocalDateTime) => Ok(Value::LocalDateTime(value)),
        (Value::LocalDateTime(value), GqlType::Date) => Ok(Value::Date(value.date())),
        (Value::LocalDateTime(value), GqlType::LocalTime) => Ok(Value::LocalTime(value.time())),
        (Value::LocalDateTime(value), GqlType::ZonedDateTime) => {
            zoned_from_local_datetime(value, ctx, span)
                .map(|value| Value::ZonedDateTime(Box::new(value)))
        }
        (Value::Date(value), GqlType::Date) => Ok(Value::Date(value)),
        (Value::Date(value), GqlType::LocalDateTime) => Ok(Value::LocalDateTime(
            value.to_datetime(jiff::civil::Time::midnight()),
        )),
        (Value::Date(value), GqlType::ZonedDateTime) => {
            zoned_from_local_datetime(value.to_datetime(jiff::civil::Time::midnight()), ctx, span)
                .map(|value| Value::ZonedDateTime(Box::new(value)))
        }
        (Value::ZonedTime(value), GqlType::ZonedTime) => Ok(Value::ZonedTime(value)),
        (Value::ZonedTime(value), GqlType::LocalDateTime) => Ok(Value::LocalDateTime(
            current_session_date(ctx).to_datetime(shifted_zoned_time_local(&value, span)?),
        )),
        (Value::ZonedTime(value), GqlType::LocalTime) => {
            shifted_zoned_time_local(&value, span).map(Value::LocalTime)
        }
        (Value::ZonedTime(value), GqlType::ZonedDateTime) => {
            let datetime = current_session_date(ctx).to_datetime(value.time());
            zoned_from_datetime_and_zone(datetime, value.offset().to_time_zone(), span)
                .map(|value| Value::ZonedDateTime(Box::new(value)))
        }
        (Value::LocalTime(value), GqlType::LocalTime) => Ok(Value::LocalTime(value)),
        (Value::LocalTime(value), GqlType::LocalDateTime) => Ok(Value::LocalDateTime(
            current_session_date(ctx).to_datetime(value),
        )),
        (Value::LocalTime(value), GqlType::ZonedDateTime) => {
            let datetime = current_session_date(ctx).to_datetime(value);
            zoned_from_local_datetime(datetime, ctx, span)
                .map(|value| Value::ZonedDateTime(Box::new(value)))
        }
        (Value::LocalTime(value), GqlType::ZonedTime) => {
            zoned_from_local_time(value, ctx, span).map(|value| Value::ZonedTime(Box::new(value)))
        }
        (Value::Duration(value), GqlType::Duration) => Ok(Value::Duration(value)),
        (Value::String(value), GqlType::ZonedDateTime) => {
            temporal_parse::parse_zoned_datetime(value.as_str().trim())
                .map(|value| Value::ZonedDateTime(Box::new(value)))
                .map_err(|error| invalid_datetime_format(error, span))
        }
        (Value::String(value), GqlType::LocalDateTime) => {
            temporal_parse::parse_local_datetime(value.as_str().trim())
                .map(Value::LocalDateTime)
                .map_err(|error| invalid_datetime_format(error, span))
        }
        (Value::String(value), GqlType::Date) => temporal_parse::parse_date(value.as_str().trim())
            .map(Value::Date)
            .map_err(|error| invalid_datetime_format(error, span)),
        (Value::String(value), GqlType::ZonedTime) => {
            temporal_parse::parse_zoned_time(value.as_str().trim())
                .map(|value| Value::ZonedTime(Box::new(value)))
                .map_err(|error| invalid_datetime_format(error, span))
        }
        (Value::String(value), GqlType::LocalTime) => {
            temporal_parse::parse_local_time(value.as_str().trim())
                .map(Value::LocalTime)
                .map_err(|error| invalid_datetime_format(error, span))
        }
        (Value::String(value), GqlType::Duration) => {
            temporal_parse::parse_duration(value.as_str().trim())
                .map(|value| Value::Duration(Box::new(value)))
                .map_err(|error| invalid_duration_format(error, span))
        }
        (source, target) => {
            Err(
                non_iso_static_source_for_target(&source, temporal_cast_target_name(target), span)
                    .unwrap_or(ExecutorError::FeatureNotSupportedYet {
                        feature: temporal_cast_to_type_feature(target),
                        span,
                    }),
            )
        }
    }
}

fn invalid_datetime_format(message: String, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(DataExceptionSubclass::InvalidDatetimeFormat, message, span)
}

fn invalid_duration_format(message: String, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(DataExceptionSubclass::InvalidDurationFormat, message, span)
}

fn zoned_from_local_datetime(
    value: jiff::civil::DateTime,
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<jiff::Zoned, ExecutorError> {
    zoned_from_datetime_and_zone(value, ctx.tx.session_time_zone().clone(), span)
}

fn zoned_from_datetime_and_zone(
    value: jiff::civil::DateTime,
    zone: jiff::tz::TimeZone,
    span: SourceSpan,
) -> Result<jiff::Zoned, ExecutorError> {
    value
        .to_zoned(zone)
        .map_err(|error| invalid_datetime_format(format!("invalid ZONED DATETIME: {error}"), span))
}

fn shifted_zoned_datetime_local(
    value: &jiff::Zoned,
    span: SourceSpan,
) -> Result<jiff::civil::DateTime, ExecutorError> {
    value
        .datetime()
        .checked_add(offset_duration(value.offset()))
        .map_err(|error| invalid_datetime_format(format!("invalid LOCAL DATETIME: {error}"), span))
}

fn shifted_zoned_time_local(
    value: &jiff::Zoned,
    span: SourceSpan,
) -> Result<jiff::civil::Time, ExecutorError> {
    let anchor = jiff::civil::Date::constant(1970, 1, 1).to_datetime(value.time());
    anchor
        .checked_add(offset_duration(value.offset()))
        .map(|datetime| datetime.time())
        .map_err(|error| invalid_datetime_format(format!("invalid LOCAL TIME: {error}"), span))
}

fn offset_duration(offset: jiff::tz::Offset) -> SignedDuration {
    SignedDuration::from_secs(i64::from(offset.seconds()))
}

fn zoned_from_local_time(
    value: jiff::civil::Time,
    ctx: &EvalCtx<'_, '_, '_, '_>,
    span: SourceSpan,
) -> Result<jiff::Zoned, ExecutorError> {
    let anchor = jiff::civil::Date::constant(1970, 1, 1).to_datetime(value);
    zoned_from_local_datetime(anchor, ctx, span)
}

fn current_session_date(ctx: &EvalCtx<'_, '_, '_, '_>) -> jiff::civil::Date {
    ctx.tx.request_timestamp_zoned().date()
}

fn temporal_cast_to_type_feature(target: &GqlType) -> &'static str {
    match target {
        GqlType::ZonedDateTime => "CAST to ZONED DATETIME",
        GqlType::LocalDateTime => "CAST to LOCAL DATETIME",
        GqlType::Date => "CAST to DATE",
        GqlType::ZonedTime => "CAST to ZONED TIME",
        GqlType::LocalTime => "CAST to LOCAL TIME",
        GqlType::Duration => "CAST to DURATION",
        _ => "CAST to temporal target",
    }
}

fn temporal_cast_target_name(target: &GqlType) -> &'static str {
    match target {
        GqlType::ZonedDateTime => "ZONED DATETIME",
        GqlType::LocalDateTime => "LOCAL DATETIME",
        GqlType::Date => "DATE",
        GqlType::ZonedTime => "ZONED TIME",
        GqlType::LocalTime => "LOCAL TIME",
        GqlType::Duration => "DURATION",
        _ => "temporal target",
    }
}
