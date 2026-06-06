//! Signed 128-bit exact numeric CAST target support.

use rust_decimal::prelude::ToPrimitive;
use selene_core::Value;

use crate::{
    SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

use super::{invalid_character, non_iso_combination, numeric_text::normalize_signed_numeric_text};

const I128_MAX_EXCLUSIVE_UPPER_BOUND: f64 = 170_141_183_460_469_231_731_687_303_715_884_105_728.0;
const I128_MIN_INCLUSIVE_LOWER_BOUND: f64 = -170_141_183_460_469_231_731_687_303_715_884_105_728.0;

pub(super) fn cast_to_int128(value: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    match value {
        Value::Int(value) => Ok(Value::Int128(i128::from(value))),
        Value::Uint(value) => Ok(Value::Int128(i128::from(value))),
        Value::Int128(value) => Ok(Value::Int128(value)),
        Value::Uint128(value) => {
            let converted = i128::try_from(value).map_err(|_| int128_out_of_range(span))?;
            Ok(Value::Int128(converted))
        }
        Value::Float(value) => float_to_int128(value, span),
        Value::Float32(value) => float_to_int128(f64::from(value), span),
        Value::Decimal(value) => decimal_to_int128(value, span),
        Value::String(value) => string_to_int128(value.as_str(), span),
        Value::Bool(_) => Err(non_iso_combination(
            "CAST from BOOLEAN to a numeric type is not a valid type combination",
            span,
        )),
        _ => Err(ExecutorError::FeatureNotSupportedYet {
            feature: "CAST source not supported for INT128 target",
            span,
        }),
    }
}

fn float_to_int128(value: f64, span: SourceSpan) -> Result<Value, ExecutorError> {
    if value.is_nan() {
        return Err(ExecutorError::data_exception(
            DataExceptionSubclass::InvalidCharacterValueForCast,
            "CAST of NaN to INT128 has no representable image",
            span,
        ));
    }
    if !value.is_finite()
        || !(I128_MIN_INCLUSIVE_LOWER_BOUND..I128_MAX_EXCLUSIVE_UPPER_BOUND).contains(&value)
    {
        return Err(int128_out_of_range(span));
    }
    let truncated = value.trunc();
    #[allow(clippy::cast_possible_truncation)]
    Ok(Value::Int128(truncated as i128))
}

fn decimal_to_int128(
    value: rust_decimal::Decimal,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    value
        .trunc()
        .to_i128()
        .map(Value::Int128)
        .ok_or_else(|| int128_out_of_range(span))
}

fn string_to_int128(text: &str, span: SourceSpan) -> Result<Value, ExecutorError> {
    let normalized = normalize_signed_numeric_text(text, "INT128", span)?;
    match normalized.parse::<i128>() {
        Ok(value) => Ok(Value::Int128(value)),
        Err(_) if integer_literal_overflows_i128(normalized.as_ref()) => {
            Err(int128_out_of_range(span))
        }
        Err(_) => Err(invalid_character(text, "INT128", span)),
    }
}

fn integer_literal_overflows_i128(text: &str) -> bool {
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
        1_u128 << 127
    } else {
        i128::MAX as u128
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

fn int128_out_of_range(span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::NumericValueOutOfRange,
        "numeric value exceeds INT128 range during CAST",
        span,
    )
}
