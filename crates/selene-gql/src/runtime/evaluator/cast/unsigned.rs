//! Unsigned exact numeric CAST target support.

use rust_decimal::prelude::ToPrimitive;
use selene_core::Value;

use crate::{
    SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

use super::{invalid_character, non_iso_combination};

#[derive(Clone, Copy)]
pub(super) enum UnsignedIntegerTarget {
    U8,
    U16,
    U32,
    U64,
    U128,
}

impl UnsignedIntegerTarget {
    const fn name(self) -> &'static str {
        match self {
            Self::U8 => "UINT8",
            Self::U16 => "UINT16",
            Self::U32 => "UINT32",
            Self::U64 => "UINT64",
            Self::U128 => "UINT128",
        }
    }

    const fn max(self) -> u128 {
        match self {
            Self::U8 => u8::MAX as u128,
            Self::U16 => u16::MAX as u128,
            Self::U32 => u32::MAX as u128,
            Self::U64 => u64::MAX as u128,
            Self::U128 => u128::MAX,
        }
    }

    const fn float_exclusive_upper_bound(self) -> f64 {
        match self {
            Self::U8 => 256.0,
            Self::U16 => 65_536.0,
            Self::U32 => 4_294_967_296.0,
            Self::U64 => 18_446_744_073_709_551_616.0,
            Self::U128 => 340_282_366_920_938_463_463_374_607_431_768_211_456.0,
        }
    }

    fn value(self, value: u128) -> Value {
        match self {
            Self::U128 => Value::Uint128(value),
            _ => {
                let narrowed =
                    u64::try_from(value).expect("target width up to UINT64 always fits in u64");
                Value::Uint(narrowed)
            }
        }
    }

    fn cast_u128(self, value: u128, span: SourceSpan) -> Result<Value, ExecutorError> {
        if value <= self.max() {
            return Ok(self.value(value));
        }
        Err(unsigned_out_of_range(self, span))
    }
}

pub(super) fn cast_to_unsigned_integer(
    value: Value,
    target: UnsignedIntegerTarget,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match value {
        Value::Int(value) => {
            let converted =
                u128::try_from(value).map_err(|_| unsigned_out_of_range(target, span))?;
            target.cast_u128(converted, span)
        }
        Value::Uint(value) => target.cast_u128(u128::from(value), span),
        Value::Int128(value) => {
            let converted =
                u128::try_from(value).map_err(|_| unsigned_out_of_range(target, span))?;
            target.cast_u128(converted, span)
        }
        Value::Uint128(value) => target.cast_u128(value, span),
        Value::Float(value) => float_to_unsigned_integer(value, target, span),
        Value::Float32(value) => float_to_unsigned_integer(f64::from(value), target, span),
        Value::Decimal(value) => decimal_to_unsigned_integer(value, target, span),
        Value::String(value) => string_to_unsigned_integer(value.as_str(), target, span),
        Value::Bool(_) => Err(non_iso_combination(
            "CAST from BOOLEAN to a numeric type is not a valid type combination",
            span,
        )),
        _ => Err(ExecutorError::FeatureNotSupportedYet {
            feature: "CAST source not supported for unsigned integer target",
            span,
        }),
    }
}

fn float_to_unsigned_integer(
    value: f64,
    target: UnsignedIntegerTarget,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    if value.is_nan() {
        return Err(ExecutorError::data_exception(
            DataExceptionSubclass::InvalidCharacterValueForCast,
            "CAST of NaN to unsigned integer has no representable image",
            span,
        ));
    }
    if !value.is_finite() || value < 0.0 || value >= target.float_exclusive_upper_bound() {
        return Err(unsigned_out_of_range(target, span));
    }
    let truncated = value.trunc();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    target.cast_u128(truncated as u128, span)
}

fn decimal_to_unsigned_integer(
    value: rust_decimal::Decimal,
    target: UnsignedIntegerTarget,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let converted = value
        .trunc()
        .to_u128()
        .ok_or_else(|| unsigned_out_of_range(target, span))?;
    target.cast_u128(converted, span)
}

fn string_to_unsigned_integer(
    text: &str,
    target: UnsignedIntegerTarget,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    text.trim()
        .parse::<u128>()
        .map_err(|_| invalid_character(text, target.name(), span))
        .and_then(|value| target.cast_u128(value, span))
}

fn unsigned_out_of_range(target: UnsignedIntegerTarget, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::NumericValueOutOfRange,
        format!("numeric value exceeds {} range during CAST", target.name()),
        span,
    )
}
