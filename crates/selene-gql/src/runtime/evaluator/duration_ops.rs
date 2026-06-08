//! Duration operator evaluation for ISO/IEC 39075:2024 §20.28.

use jiff::{Span, SpanRelativeTo};
use selene_core::Value;

use crate::{
    BinaryOp, SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

const NANOS_PER_SECOND: i128 = 1_000_000_000;
const NANOS_PER_MILLISECOND: i128 = 1_000_000;
const NANOS_PER_MICROSECOND: i128 = 1_000;
const NANOS_PER_MINUTE: i128 = 60 * NANOS_PER_SECOND;
const NANOS_PER_HOUR: i128 = 60 * NANOS_PER_MINUTE;
const NANOS_PER_DAY: i128 = 24 * NANOS_PER_HOUR;
const NANOS_PER_WEEK: i128 = 7 * NANOS_PER_DAY;

pub(super) fn eval_arithmetic(
    op: BinaryOp,
    lhs: Span,
    rhs: Span,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    if !matches!(op, BinaryOp::Add | BinaryOp::Sub) {
        return Err(ExecutorError::data_exception(
            DataExceptionSubclass::InvalidValueType,
            "duration operands only support addition and subtraction",
            span,
        ));
    }

    let Some(group) = operation_group(&lhs, &rhs) else {
        return Err(ExecutorError::data_exception(
            DataExceptionSubclass::IncompatibleTemporalInstantUnitGroups,
            "duration operands use incompatible temporal instant unit groups",
            span,
        ));
    };
    let duration = match group {
        DurationUnitGroup::Zero => Ok(Span::new()),
        DurationUnitGroup::YearMonth => year_month_arithmetic(op, &lhs, &rhs, span),
        DurationUnitGroup::DayTime => day_time_arithmetic(op, &lhs, &rhs, span),
    }?;
    Ok(Value::Duration(Box::new(duration)))
}

pub(super) fn eval_scaling(
    op: BinaryOp,
    duration: Span,
    coefficient: f64,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    if !matches!(op, BinaryOp::Mul | BinaryOp::Div) {
        return Err(ExecutorError::data_exception(
            DataExceptionSubclass::InvalidValueType,
            "duration operands only support multiplication and division by a numeric coefficient",
            span,
        ));
    }
    if !coefficient.is_finite() {
        return Err(overflow(span));
    }
    let factor = match op {
        BinaryOp::Mul => coefficient,
        BinaryOp::Div if coefficient == 0.0 => {
            return Err(ExecutorError::data_exception(
                DataExceptionSubclass::DivisionByZero,
                "duration division by zero",
                span,
            ));
        }
        BinaryOp::Div => 1.0 / coefficient,
        _ => unreachable!("guarded by eval_scaling"),
    };
    if !factor.is_finite() {
        return Err(overflow(span));
    }

    let Some(group) = unit_group(&duration) else {
        return Err(ExecutorError::data_exception(
            DataExceptionSubclass::IncompatibleTemporalInstantUnitGroups,
            "duration operands use incompatible temporal instant unit groups",
            span,
        ));
    };
    let duration = match group {
        DurationUnitGroup::Zero => Ok(Span::new()),
        DurationUnitGroup::YearMonth => scale_year_month(&duration, factor, span),
        DurationUnitGroup::DayTime => scale_day_time(&duration, factor, span),
    }?;
    Ok(Value::Duration(Box::new(duration)))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DurationUnitGroup {
    Zero,
    YearMonth,
    DayTime,
}

fn operation_group(lhs: &Span, rhs: &Span) -> Option<DurationUnitGroup> {
    match (unit_group(lhs)?, unit_group(rhs)?) {
        (DurationUnitGroup::Zero, DurationUnitGroup::Zero) => Some(DurationUnitGroup::Zero),
        (DurationUnitGroup::Zero, group) | (group, DurationUnitGroup::Zero) => Some(group),
        (DurationUnitGroup::YearMonth, DurationUnitGroup::YearMonth) => {
            Some(DurationUnitGroup::YearMonth)
        }
        (DurationUnitGroup::DayTime, DurationUnitGroup::DayTime) => {
            Some(DurationUnitGroup::DayTime)
        }
        (DurationUnitGroup::YearMonth, DurationUnitGroup::DayTime)
        | (DurationUnitGroup::DayTime, DurationUnitGroup::YearMonth) => None,
    }
}

fn unit_group(value: &Span) -> Option<DurationUnitGroup> {
    let has_year_month = value.get_years() != 0 || value.get_months() != 0;
    let has_day_time = value.get_weeks() != 0
        || value.get_days() != 0
        || value.get_hours() != 0
        || value.get_minutes() != 0
        || value.get_seconds() != 0
        || value.get_milliseconds() != 0
        || value.get_microseconds() != 0
        || value.get_nanoseconds() != 0;
    match (has_year_month, has_day_time) {
        (false, false) => Some(DurationUnitGroup::Zero),
        (true, false) => Some(DurationUnitGroup::YearMonth),
        (false, true) => Some(DurationUnitGroup::DayTime),
        (true, true) => None,
    }
}

fn year_month_arithmetic(
    op: BinaryOp,
    lhs: &Span,
    rhs: &Span,
    span: SourceSpan,
) -> Result<Span, ExecutorError> {
    let lhs_months = total_year_months(lhs);
    let rhs_months = total_year_months(rhs);
    let rhs_months = if op == BinaryOp::Sub {
        rhs_months.checked_neg().ok_or_else(|| overflow(span))?
    } else {
        rhs_months
    };
    let total = lhs_months
        .checked_add(rhs_months)
        .ok_or_else(|| overflow(span))?;
    span_from_total_months(total, span)
}

fn total_year_months(value: &Span) -> i64 {
    i64::from(value.get_years()) * 12 + i64::from(value.get_months())
}

fn span_from_total_months(total: i64, span: SourceSpan) -> Result<Span, ExecutorError> {
    if total == 0 {
        return Ok(Span::new());
    }
    let sign = if total < 0 { "-" } else { "" };
    let abs = total.unsigned_abs();
    let years = abs / 12;
    let months = abs % 12;
    let text = match (years, months) {
        (0, months) => format!("{sign}P{months}M"),
        (years, 0) => format!("{sign}P{years}Y"),
        (years, months) => format!("{sign}P{years}Y{months}M"),
    };
    text.parse().map_err(|error| {
        ExecutorError::data_exception(
            DataExceptionSubclass::NumericValueOutOfRange,
            format!("duration arithmetic result is out of range: {error}"),
            span,
        )
    })
}

fn scale_year_month(value: &Span, factor: f64, span: SourceSpan) -> Result<Span, ExecutorError> {
    let total = total_year_months(value) as f64;
    let scaled = total * factor;
    if !scaled.is_finite() {
        return Err(overflow(span));
    }
    let whole_months = scaled.trunc();
    if !is_effectively_integral(scaled, whole_months) {
        return Err(ExecutorError::data_exception(
            DataExceptionSubclass::NumericValueOutOfRange,
            "duration scaling produced fractional months",
            span,
        ));
    }
    if whole_months < i64::MIN as f64 || whole_months > i64::MAX as f64 {
        return Err(overflow(span));
    }
    span_from_total_months(whole_months as i64, span)
}

fn is_effectively_integral(value: f64, truncated: f64) -> bool {
    (value - truncated).abs() <= f64::EPSILON * value.abs().max(1.0)
}

fn day_time_arithmetic(
    op: BinaryOp,
    lhs: &Span,
    rhs: &Span,
    span: SourceSpan,
) -> Result<Span, ExecutorError> {
    let relative = SpanRelativeTo::days_are_24_hours();
    let result = match op {
        BinaryOp::Add => lhs.checked_add((*rhs, relative)),
        BinaryOp::Sub => lhs.checked_sub((*rhs, relative)),
        _ => unreachable!("guarded by eval_arithmetic"),
    };
    result.map_err(|error| {
        ExecutorError::data_exception(
            DataExceptionSubclass::NumericValueOutOfRange,
            format!("duration arithmetic result is out of range: {error}"),
            span,
        )
    })
}

fn scale_day_time(value: &Span, factor: f64, span: SourceSpan) -> Result<Span, ExecutorError> {
    let total = total_day_time_nanos(value, span)?;
    let scaled = scale_i128_by_f64(total, factor, span)?;
    span_from_total_nanos(scaled, span)
}

fn total_day_time_nanos(value: &Span, span: SourceSpan) -> Result<i128, ExecutorError> {
    let terms = [
        (i128::from(value.get_weeks()), NANOS_PER_WEEK),
        (i128::from(value.get_days()), NANOS_PER_DAY),
        (i128::from(value.get_hours()), NANOS_PER_HOUR),
        (i128::from(value.get_minutes()), NANOS_PER_MINUTE),
        (i128::from(value.get_seconds()), NANOS_PER_SECOND),
        (i128::from(value.get_milliseconds()), NANOS_PER_MILLISECOND),
        (i128::from(value.get_microseconds()), NANOS_PER_MICROSECOND),
        (i128::from(value.get_nanoseconds()), 1),
    ];
    let mut total = 0_i128;
    for (quantity, nanos) in terms {
        let term = quantity.checked_mul(nanos).ok_or_else(|| overflow(span))?;
        total = total.checked_add(term).ok_or_else(|| overflow(span))?;
    }
    Ok(total)
}

fn scale_i128_by_f64(value: i128, factor: f64, span: SourceSpan) -> Result<i128, ExecutorError> {
    if value == 0 || factor == 0.0 {
        return Ok(0);
    }
    let scaled = (value as f64) * factor;
    if !scaled.is_finite() {
        return Err(overflow(span));
    }
    let truncated = scaled.trunc();
    if truncated < i128::MIN as f64 || truncated > i128::MAX as f64 {
        return Err(overflow(span));
    }
    Ok(truncated as i128)
}

fn span_from_total_nanos(total: i128, span: SourceSpan) -> Result<Span, ExecutorError> {
    if total == 0 {
        return Ok(Span::new());
    }
    let negative = total < 0;
    let mut remaining = total.unsigned_abs();
    let days = remaining / NANOS_PER_DAY as u128;
    remaining %= NANOS_PER_DAY as u128;
    let hours = remaining / NANOS_PER_HOUR as u128;
    remaining %= NANOS_PER_HOUR as u128;
    let minutes = remaining / NANOS_PER_MINUTE as u128;
    remaining %= NANOS_PER_MINUTE as u128;
    let seconds = remaining / NANOS_PER_SECOND as u128;
    remaining %= NANOS_PER_SECOND as u128;
    let nanoseconds = remaining;

    let mut value = Span::new();
    value = value
        .try_days(signed_component(days, negative, span)?)
        .map_err(|error| duration_range_error(error, span))?;
    value = value
        .try_hours(signed_component(hours, negative, span)?)
        .map_err(|error| duration_range_error(error, span))?;
    value = value
        .try_minutes(signed_component(minutes, negative, span)?)
        .map_err(|error| duration_range_error(error, span))?;
    value = value
        .try_seconds(signed_component(seconds, negative, span)?)
        .map_err(|error| duration_range_error(error, span))?;
    value
        .try_nanoseconds(signed_component(nanoseconds, negative, span)?)
        .map_err(|error| duration_range_error(error, span))
}

fn signed_component(value: u128, negative: bool, span: SourceSpan) -> Result<i64, ExecutorError> {
    let value = i64::try_from(value).map_err(|_| overflow(span))?;
    if negative {
        value.checked_neg().ok_or_else(|| overflow(span))
    } else {
        Ok(value)
    }
}

fn overflow(span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::NumericValueOutOfRange,
        "duration arithmetic result is out of range",
        span,
    )
}

fn duration_range_error(error: jiff::Error, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::NumericValueOutOfRange,
        format!("duration arithmetic result is out of range: {error}"),
        span,
    )
}
