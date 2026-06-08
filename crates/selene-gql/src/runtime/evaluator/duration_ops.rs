//! Duration operator evaluation for ISO/IEC 39075:2024 §20.28.

use jiff::{Span, SpanRelativeTo};
use selene_core::Value;

use crate::{
    BinaryOp, SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

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

fn overflow(span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::NumericValueOutOfRange,
        "duration arithmetic result is out of range",
        span,
    )
}
