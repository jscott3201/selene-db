//! Signed exact numeric CAST target support up to the canonical INTEGER value.

use selene_core::Value;

use crate::{
    SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

use super::{
    decimal, invalid_character, non_iso_combination, non_iso_static_source_for_target,
    numeric_text::{NumericText, classify_signed_numeric_text},
};

#[derive(Clone, Copy)]
pub(super) enum SignedIntegerTarget {
    I8,
    I16,
    I32,
    I64,
}

impl SignedIntegerTarget {
    const fn name(self) -> &'static str {
        match self {
            Self::I8 => "INT8",
            Self::I16 => "INT16",
            Self::I32 => "INT32",
            Self::I64 => "INTEGER",
        }
    }

    fn contains(self, value: i64) -> bool {
        match self {
            Self::I8 => i8::try_from(value).is_ok(),
            Self::I16 => i16::try_from(value).is_ok(),
            Self::I32 => i32::try_from(value).is_ok(),
            Self::I64 => true,
        }
    }
}

pub(super) fn cast_to_signed_integer(
    value: Value,
    target: SignedIntegerTarget,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let Value::Int(value) = cast_to_integer(value, target, span)? else {
        unreachable!("cast_to_integer returns Value::Int on success");
    };
    if target.contains(value) {
        return Ok(Value::Int(value));
    }
    Err(ExecutorError::data_exception(
        DataExceptionSubclass::NumericValueOutOfRange,
        format!("INTEGER value exceeds {} range during CAST", target.name()),
        span,
    ))
}

fn cast_to_integer(
    value: Value,
    target: SignedIntegerTarget,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    // Per ISO §20.8 Table 4 the integer target is `EN`; every numeric source
    // family (`EN`/`UN`/`AN`) is a `Y` cell. Exact-integer sources widen to
    // their natural intermediate (`u64`/`i128`/`u128`) with an explicit i64
    // range check. A boolean source is Table-4 `N` -> 22G03.
    match value {
        Value::Int(value) => Ok(Value::Int(value)),
        Value::Uint(value) => decimal::u64_to_int(value, span),
        Value::Int128(value) => decimal::i128_to_int(value, span),
        Value::Uint128(value) => decimal::u128_to_int(value, span),
        Value::Float(value) => float_to_integer(value, span),
        Value::Float32(value) => float_to_integer(f64::from(value), span),
        Value::Decimal(value) => decimal::decimal_to_int(value, span),
        Value::String(value) => string_to_integer(value.as_str(), span),
        Value::Bool(_) => Err(non_iso_combination(
            "CAST from BOOLEAN to a numeric type is not a valid type combination",
            span,
        )),
        other => Err(
            non_iso_static_source_for_target(&other, target.name(), span).unwrap_or(
                ExecutorError::FeatureNotSupportedYet {
                    feature: "CAST source not supported for INTEGER target",
                    span,
                },
            ),
        ),
    }
}

fn float_to_integer(value: f64, span: SourceSpan) -> Result<Value, ExecutorError> {
    if value.is_nan() {
        return Err(ExecutorError::data_exception(
            DataExceptionSubclass::InvalidCharacterValueForCast,
            "CAST of NaN to INTEGER has no representable image",
            span,
        ));
    }
    // Rust `as` saturates rather than wrapping, so an explicit range check
    // enforces the 22003 contract before conversion.
    #[allow(clippy::cast_precision_loss)]
    let max = i64::MAX as f64;
    #[allow(clippy::cast_precision_loss)]
    let min = i64::MIN as f64;
    if !value.is_finite() || value >= max || value <= min {
        return Err(signed_out_of_range(
            span,
            "FLOAT value exceeds INTEGER range during CAST",
        ));
    }
    let truncated = value.trunc();
    #[allow(clippy::cast_possible_truncation)]
    Ok(Value::Int(truncated as i64))
}

fn string_to_integer(text: &str, span: SourceSpan) -> Result<Value, ExecutorError> {
    match classify_signed_numeric_text(text, "INTEGER", span)? {
        NumericText::Integer(image) => string_integer_text_to_integer(image.as_ref(), text, span),
        NumericText::Decimal(image) => image
            .parse::<rust_decimal::Decimal>()
            .map_err(|_| invalid_character(text, "INTEGER", span))
            .and_then(|value| decimal::decimal_to_int(value, span)),
        NumericText::Approximate(image) => image
            .parse::<f64>()
            .map_err(|_| invalid_character(text, "INTEGER", span))
            .and_then(|value| float_to_integer(value, span)),
    }
}

fn string_integer_text_to_integer(
    normalized: &str,
    original: &str,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match normalized.parse::<i64>() {
        Ok(value) => Ok(Value::Int(value)),
        Err(_) if signed_integer_literal_overflows_i64(normalized) => Err(signed_out_of_range(
            span,
            "STRING value exceeds INTEGER range during CAST",
        )),
        Err(_) => Err(invalid_character(original, "INTEGER", span)),
    }
}

fn signed_integer_literal_overflows_i64(text: &str) -> bool {
    let (negative, digits) = match text.as_bytes().first().copied() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        Some(_) => (false, text),
        None => return false,
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let limit = if negative {
        1_u128 << 63
    } else {
        i64::MAX as u128
    };
    digits_exceed_u128_limit(digits, limit)
}

fn digits_exceed_u128_limit(digits: &str, limit: u128) -> bool {
    let mut value = 0_u128;
    for byte in digits.bytes() {
        let digit = u128::from(byte - b'0');
        if value > (limit - digit) / 10 {
            return true;
        }
        value = value * 10 + digit;
    }
    false
}

fn signed_out_of_range(span: SourceSpan, message: &'static str) -> ExecutorError {
    ExecutorError::data_exception(DataExceptionSubclass::NumericValueOutOfRange, message, span)
}
