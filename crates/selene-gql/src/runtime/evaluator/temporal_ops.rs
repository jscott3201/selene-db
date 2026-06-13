//! Temporal instant plus duration operator evaluation for ISO/IEC 39075:2024 §20.26.

use jiff::Span;
use selene_core::Value;

use crate::{
    BinaryOp, SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

pub(super) fn eval_duration_plus_temporal(
    duration: Span,
    instant: Value,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    eval_temporal_duration(BinaryOp::Add, instant, duration, span)
}

pub(super) fn eval_temporal_duration(
    op: BinaryOp,
    instant: Value,
    duration: Span,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match op {
        BinaryOp::Add => add_duration(instant, &duration, span),
        BinaryOp::Sub => subtract_duration(instant, &duration, span),
        _ => Err(ExecutorError::data_exception(
            DataExceptionSubclass::InvalidValueType,
            "temporal instant operands only support addition and subtraction by duration",
            span,
        )),
    }
}

fn add_duration(instant: Value, duration: &Span, span: SourceSpan) -> Result<Value, ExecutorError> {
    match instant {
        Value::ZonedDateTime(value) => value
            .checked_add(duration)
            .map(|value| Value::ZonedDateTime(Box::new(value)))
            .map_err(|error| range_error(error, span)),
        Value::LocalDateTime(value) => value
            .checked_add(duration)
            .map(Value::LocalDateTime)
            .map_err(|error| range_error(error, span)),
        Value::Date(value) => value
            .checked_add(duration)
            .map(Value::Date)
            .map_err(|error| range_error(error, span)),
        Value::ZonedTime(value) => value
            .checked_add(duration)
            .map(|value| Value::ZonedTime(Box::new(value)))
            .map_err(|error| range_error(error, span)),
        Value::LocalTime(value) => value
            .checked_add(duration)
            .map(Value::LocalTime)
            .map_err(|error| range_error(error, span)),
        _ => Err(invalid_temporal(span)),
    }
}

fn subtract_duration(
    instant: Value,
    duration: &Span,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match instant {
        Value::ZonedDateTime(value) => value
            .checked_sub(duration)
            .map(|value| Value::ZonedDateTime(Box::new(value)))
            .map_err(|error| range_error(error, span)),
        Value::LocalDateTime(value) => value
            .checked_sub(duration)
            .map(Value::LocalDateTime)
            .map_err(|error| range_error(error, span)),
        Value::Date(value) => value
            .checked_sub(duration)
            .map(Value::Date)
            .map_err(|error| range_error(error, span)),
        Value::ZonedTime(value) => value
            .checked_sub(duration)
            .map(|value| Value::ZonedTime(Box::new(value)))
            .map_err(|error| range_error(error, span)),
        Value::LocalTime(value) => value
            .checked_sub(duration)
            .map(Value::LocalTime)
            .map_err(|error| range_error(error, span)),
        _ => Err(invalid_temporal(span)),
    }
}

fn invalid_temporal(span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::InvalidValueType,
        "duration arithmetic operand is not a temporal instant",
        span,
    )
}

fn range_error(error: jiff::Error, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::NumericValueOutOfRange,
        format!("temporal instant arithmetic result is out of range: {error}"),
        span,
    )
}
