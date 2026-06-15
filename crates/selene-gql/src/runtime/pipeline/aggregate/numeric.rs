//! Numeric aggregate accumulators and conversions.

use rust_decimal::prelude::ToPrimitive;
use selene_core::Value;

use crate::{
    SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError, value_compare},
};

/// Running accumulator for `SUM` / `AVG`.
///
/// The variants form a widening lattice that mirrors `eval_arithmetic`'s
/// promotion order: `Int` (i64) widens to `Int128` (i128) on i64 overflow or
/// when mixed with a 128-bit operand; `Decimal` is the exact base-10 channel
/// for `DECIMAL` inputs (and integers mixed with them); `Float` (f64) is the
/// lossy top reached as soon as any binary float participates. 128-bit and
/// DECIMAL inputs keep `GV13`/`GV14`/`GV17` honest end-to-end (GQLRT-27).
#[derive(Clone)]
pub(super) enum NumericSum {
    Int(i64),
    Int128(i128),
    Decimal(rust_decimal::Decimal),
    Float(f64),
}

impl NumericSum {
    pub(super) fn into_value(self) -> Value {
        match self {
            Self::Int(value) => Value::Int(value),
            Self::Int128(value) => Value::Int128(value),
            Self::Decimal(value) => Value::Decimal(value),
            Self::Float(value) => Value::Float(value),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Welford {
    count: u64,
    mean: f64,
    m2: f64,
}

impl Welford {
    pub(super) fn observe(&mut self, value: Value, span: SourceSpan) -> Result<(), ExecutorError> {
        let value = numeric_sum_to_f64(numeric_value(value, span)?, span)?;
        let count = self.count.checked_add(1).ok_or_else(|| {
            data_exception_value(
                DataExceptionSubclass::NumericValueOutOfRange,
                "aggregate count is out of range",
                span,
            )
        })?;
        let delta = value - self.mean;
        self.count = count;
        self.mean += delta / count as f64;
        let delta2 = value - self.mean;
        self.m2 = finite_float(self.m2 + delta * delta2, span)?;
        Ok(())
    }
}

pub(super) fn add_numeric(
    current: Option<NumericSum>,
    value: Value,
    span: SourceSpan,
) -> Result<NumericSum, ExecutorError> {
    let next = numeric_value(value, span)?;
    match (current, next) {
        (None, next) => Ok(next),
        (Some(NumericSum::Int(lhs)), NumericSum::Int(rhs)) => match lhs.checked_add(rhs) {
            Some(value) => Ok(NumericSum::Int(value)),
            // i64 overflow widens to i128 rather than failing outright.
            None => add_i128(i128::from(lhs), i128::from(rhs), span),
        },
        // Integer family (i64 + i128 in any order) accumulates in i128.
        (Some(NumericSum::Int(lhs)), NumericSum::Int128(rhs))
        | (Some(NumericSum::Int128(rhs)), NumericSum::Int(lhs)) => {
            add_i128(i128::from(lhs), rhs, span)
        }
        (Some(NumericSum::Int128(lhs)), NumericSum::Int128(rhs)) => add_i128(lhs, rhs, span),
        // Decimal channel: Decimal mixed with the integer family stays exact.
        (Some(NumericSum::Decimal(lhs)), NumericSum::Decimal(rhs)) => add_decimal(lhs, rhs, span),
        (Some(NumericSum::Decimal(lhs)), NumericSum::Int(rhs))
        | (Some(NumericSum::Int(rhs)), NumericSum::Decimal(lhs)) => {
            add_decimal(lhs, rust_decimal::Decimal::from(rhs), span)
        }
        (Some(NumericSum::Decimal(lhs)), NumericSum::Int128(rhs))
        | (Some(NumericSum::Int128(rhs)), NumericSum::Decimal(lhs)) => {
            let rhs = rust_decimal::Decimal::try_from_i128_with_scale(rhs, 0).map_err(|_| {
                data_exception_value(
                    DataExceptionSubclass::NumericValueOutOfRange,
                    "128-bit aggregate value exceeds DECIMAL range",
                    span,
                )
            })?;
            add_decimal(lhs, rhs, span)
        }
        // Any float participant collapses the running sum to f64.
        (Some(lhs), rhs) => {
            let lhs = numeric_sum_to_f64(lhs, span)?;
            let rhs = numeric_sum_to_f64(rhs, span)?;
            finite_float(lhs + rhs, span).map(NumericSum::Float)
        }
    }
}

fn add_i128(lhs: i128, rhs: i128, span: SourceSpan) -> Result<NumericSum, ExecutorError> {
    lhs.checked_add(rhs).map(NumericSum::Int128).ok_or_else(|| {
        data_exception_value(
            DataExceptionSubclass::NumericValueOutOfRange,
            "integer aggregate overflow",
            span,
        )
    })
}

fn add_decimal(
    lhs: rust_decimal::Decimal,
    rhs: rust_decimal::Decimal,
    span: SourceSpan,
) -> Result<NumericSum, ExecutorError> {
    lhs.checked_add(rhs)
        .map(NumericSum::Decimal)
        .ok_or_else(|| {
            data_exception_value(
                DataExceptionSubclass::NumericValueOutOfRange,
                "decimal aggregate overflow",
                span,
            )
        })
}

fn numeric_value(value: Value, span: SourceSpan) -> Result<NumericSum, ExecutorError> {
    match value {
        Value::Int(value) => Ok(NumericSum::Int(value)),
        Value::Uint(value) => Ok(i64::try_from(value)
            .map_or_else(|_| NumericSum::Int128(i128::from(value)), NumericSum::Int)),
        Value::Int128(value) => Ok(NumericSum::Int128(value)),
        Value::Uint128(value) => i128::try_from(value).map(NumericSum::Int128).map_err(|_| {
            data_exception_value(
                DataExceptionSubclass::NumericValueOutOfRange,
                "unsigned 128-bit aggregate value is out of range",
                span,
            )
        }),
        Value::Decimal(value) => Ok(NumericSum::Decimal(value)),
        Value::Float(value) => finite_float(value, span).map(NumericSum::Float),
        Value::Float32(value) => finite_float(f64::from(value), span).map(NumericSum::Float),
        _ => Err(data_exception_value(
            DataExceptionSubclass::InvalidValueType,
            "aggregate value is not numeric",
            span,
        )),
    }
}

pub(super) fn percentile_value(
    value: Value,
    span: SourceSpan,
) -> Result<Option<f64>, ExecutorError> {
    if matches!(value, Value::Null) {
        return Ok(None);
    }
    let percentile = percentile_numeric_to_f64(&value, span)?;
    if (0.0..=1.0).contains(&percentile) {
        Ok(Some(percentile))
    } else {
        Err(data_exception_value(
            DataExceptionSubclass::NumericValueOutOfRange,
            "PERCENTILE independent value must be between 0 and 1",
            span,
        ))
    }
}

pub(super) fn percentile_numeric_to_f64(
    value: &Value,
    span: SourceSpan,
) -> Result<f64, ExecutorError> {
    let value = match value {
        Value::Int(value) => *value as f64,
        Value::Uint(value) => *value as f64,
        Value::Int128(value) => *value as f64,
        Value::Uint128(value) => *value as f64,
        Value::Decimal(value) => {
            return value.to_f64().ok_or_else(|| {
                data_exception_value(
                    DataExceptionSubclass::NumericValueOutOfRange,
                    "decimal percentile value is out of float range",
                    span,
                )
            });
        }
        Value::Float(value) => *value,
        Value::Float32(value) => f64::from(*value),
        _ => {
            return Err(data_exception_value(
                DataExceptionSubclass::InvalidValueType,
                "percentile value is not numeric",
                span,
            ));
        }
    };
    finite_float(value, span)
}

fn numeric_sum_to_f64(value: NumericSum, span: SourceSpan) -> Result<f64, ExecutorError> {
    match value {
        NumericSum::Int(value) => i64_to_f64_exact(value).ok_or_else(|| {
            data_exception_value(
                DataExceptionSubclass::NumericValueOutOfRange,
                "integer aggregate value is not exactly float-representable",
                span,
            )
        }),
        NumericSum::Int128(value) => i128_to_f64_exact(value).ok_or_else(|| {
            data_exception_value(
                DataExceptionSubclass::NumericValueOutOfRange,
                "128-bit aggregate value is not exactly float-representable",
                span,
            )
        }),
        // DECIMAL → f64 is intrinsically lossy; STDDEV and the float-collapse
        // SUM path accept the nearest f64.
        NumericSum::Decimal(value) => value.to_f64().ok_or_else(|| {
            data_exception_value(
                DataExceptionSubclass::NumericValueOutOfRange,
                "decimal aggregate value is out of float range",
                span,
            )
        }),
        NumericSum::Float(value) => Ok(value),
    }
}

pub(super) fn avg_to_value(
    sum: Option<NumericSum>,
    count: u64,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let Some(sum) = sum else {
        return Ok(Value::Null);
    };
    if count == 0 {
        return Ok(Value::Null);
    }
    // DECIMAL averages stay in the exact base-10 channel; every other numeric
    // channel divides in f64 (ISO `AVG` returns an approximate numeric).
    if let NumericSum::Decimal(sum) = sum
        && let Some(avg) = sum.checked_div(rust_decimal::Decimal::from(count))
    {
        return Ok(Value::Decimal(avg));
    }
    let sum = numeric_sum_to_f64(sum, span)?;
    finite_float(sum / count as f64, span).map(Value::Float)
}

pub(super) fn stddev_pop_to_value(
    stats: Welford,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    if stats.count == 0 {
        return Ok(Value::Null);
    }
    finite_float((stats.m2 / stats.count as f64).sqrt(), span).map(Value::Float)
}

pub(super) fn stddev_samp_to_value(
    stats: Welford,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    if stats.count < 2 {
        return Ok(Value::Null);
    }
    finite_float((stats.m2 / (stats.count - 1) as f64).sqrt(), span).map(Value::Float)
}

pub(super) fn percentile_cont_to_value(
    mut values: Vec<f64>,
    percentile: Option<Option<f64>>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let Some(Some(percentile)) = percentile else {
        return Ok(Value::Null);
    };
    if values.is_empty() {
        return Ok(Value::Null);
    }
    values.sort_by(f64::total_cmp);
    if values.len() == 1 {
        return finite_float(values[0], span).map(Value::Float);
    }
    let index = 1.0 + percentile * (values.len() as f64 - 1.0);
    let floor = index.floor();
    let ceil = index.ceil();
    let floor_value = values[floor as usize - 1];
    let ceil_value = values[ceil as usize - 1];
    let value = if floor == ceil {
        floor_value
    } else {
        (ceil - index) * floor_value + (index - floor) * ceil_value
    };
    finite_float(value, span).map(Value::Float)
}

pub(super) fn percentile_disc_to_value(
    mut values: Vec<Value>,
    percentile: Option<Option<f64>>,
    _span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let Some(Some(percentile)) = percentile else {
        return Ok(Value::Null);
    };
    if values.is_empty() {
        return Ok(Value::Null);
    }
    values.sort_by(|lhs, rhs| {
        value_compare::compare_non_null(lhs, rhs)
            .expect("PERCENTILE_DISC values are validated as comparable numeric values")
    });
    let index = 1.0 + percentile * (values.len() as f64 - 1.0);
    let rounded = index.round_ties_even().clamp(1.0, values.len() as f64);
    Ok(values[rounded as usize - 1].clone())
}

fn finite_float(value: f64, span: SourceSpan) -> Result<f64, ExecutorError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(data_exception_value(
            DataExceptionSubclass::NumericValueOutOfRange,
            "floating-point aggregate produced non-finite value",
            span,
        ))
    }
}

pub(super) fn count_to_value(count: u64, span: SourceSpan) -> Result<Value, ExecutorError> {
    i64::try_from(count).map(Value::Int).map_err(|_| {
        data_exception_value(
            DataExceptionSubclass::NumericValueOutOfRange,
            "aggregate count is out of range",
            span,
        )
    })
}

fn i64_to_f64_exact(value: i64) -> Option<f64> {
    u128_representable_by_binary_float(u128::from(value.unsigned_abs()), 53).then_some(value as f64)
}

fn i128_to_f64_exact(value: i128) -> Option<f64> {
    u128_representable_by_binary_float(value.unsigned_abs(), 53).then_some(value as f64)
}

fn u128_representable_by_binary_float(value: u128, significand_bits: u32) -> bool {
    if value == 0 {
        return true;
    }
    let exponent = u128::BITS - 1 - value.leading_zeros();
    if exponent < significand_bits {
        return true;
    }
    let low_bits = exponent + 1 - significand_bits;
    let mask = (1_u128 << low_bits) - 1;
    value & mask == 0
}

pub(super) fn data_exception_value(
    subclass: DataExceptionSubclass,
    message: impl Into<String>,
    span: SourceSpan,
) -> ExecutorError {
    ExecutorError::data_exception(subclass, message, span)
}
