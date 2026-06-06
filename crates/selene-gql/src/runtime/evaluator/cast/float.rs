//! Approximate numeric CAST target support.

use rust_decimal::prelude::ToPrimitive;
use selene_core::Value;

use crate::{
    SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

use super::{invalid_character, non_iso_combination};

#[derive(Clone, Copy)]
pub(super) enum FloatTarget {
    F32,
    F64,
}

impl FloatTarget {
    const fn name(self) -> &'static str {
        match self {
            Self::F32 => "FLOAT32",
            Self::F64 => "FLOAT",
        }
    }

    fn value(self, value: f64, span: SourceSpan) -> Result<Value, ExecutorError> {
        match self {
            Self::F64 => Ok(Value::Float(value)),
            Self::F32 => {
                #[allow(clippy::cast_possible_truncation)]
                let narrowed = value as f32;
                if value.is_finite() && !narrowed.is_finite() {
                    return Err(float32_out_of_range(span));
                }
                Ok(Value::Float32(narrowed))
            }
        }
    }
}

pub(super) fn cast_to_float(
    value: Value,
    target: FloatTarget,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    // Per ISO §20.8 Table 4 the approximate target is `AN`; every numeric
    // source is a `Y` cell (GR4i). Loss of least-significant precision is
    // permitted (round/truncate, IA005), but a finite source that cannot be
    // represented in FLOAT32 still loses leading significant digits -> 22003.
    match value {
        Value::Float(value) => target.value(value, span),
        // Exact-integer -> float is lossy above 2^53, but GR4i permits that
        // least-significant precision loss. Convert through f64, then narrow
        // only when the declared target is FLOAT32.
        #[allow(clippy::cast_precision_loss)]
        Value::Int(value) => target.value(value as f64, span),
        #[allow(clippy::cast_precision_loss)]
        Value::Uint(value) => target.value(value as f64, span),
        #[allow(clippy::cast_precision_loss)]
        Value::Int128(value) => target.value(value as f64, span),
        #[allow(clippy::cast_precision_loss)]
        Value::Uint128(value) => target.value(value as f64, span),
        Value::Float32(value) => target.value(f64::from(value), span),
        Value::Decimal(value) => decimal_to_float(value, target, span),
        Value::String(value) => string_to_float(value.as_str(), target, span),
        Value::Bool(_) => Err(non_iso_combination(
            "CAST from BOOLEAN to a numeric type is not a valid type combination",
            span,
        )),
        _ => Err(ExecutorError::FeatureNotSupportedYet {
            feature: "CAST source not supported for FLOAT target",
            span,
        }),
    }
}

fn decimal_to_float(
    value: rust_decimal::Decimal,
    target: FloatTarget,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    value
        .to_f64()
        .ok_or_else(|| float_out_of_range(span))
        .and_then(|value| target.value(value, span))
}

fn string_to_float(
    text: &str,
    target: FloatTarget,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    // Trim whitespace per ecosystem precedent (Postgres/Neo4j/SQLite); see
    // BRIEF-135a §O Q2-deviation.
    text.trim()
        .parse::<f64>()
        .map_err(|_| invalid_character(text, target.name(), span))
        .and_then(|value| target.value(value, span))
}

fn float_out_of_range(span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::NumericValueOutOfRange,
        "DECIMAL value has no FLOAT image during CAST",
        span,
    )
}

fn float32_out_of_range(span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::NumericValueOutOfRange,
        "numeric value exceeds FLOAT32 range during CAST",
        span,
    )
}
