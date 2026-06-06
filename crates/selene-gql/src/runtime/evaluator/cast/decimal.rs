//! DECIMAL CAST arms + numeric-family source widening for ISO/IEC 39075:2024
//! §20.8 explicit `CAST`.
//!
//! `DECIMAL` is a **signed exact numeric** type (Table 4 `EN`), so every
//! `EN ↔ EN/UN/AN/C` cell involving `DECIMAL` is a Table-4 `Y` (mandated,
//! gated only by GA05 + GV17, both already flagged). This module owns:
//!
//! - `numeric_to_decimal` — `{Int,Uint,Int128,Uint128,Float,Float32}` → DECIMAL
//!   (`22003` on Decimal-range overflow) and `String` → DECIMAL (`22018` on
//!   parse failure).
//! - `decimal_to_int` / `decimal_to_string` — DECIMAL → the canonical integer
//!   (`Value::Int(i64)`) and string targets. Decimal-to-approximate targets live
//!   in the `float` submodule because `FLOAT32` and `FLOAT64` have different
//!   runtime value shapes. Integer truncates toward zero per §20.8 GR4g(i) /
//!   IA005, raising `22003` on loss of a leading significant digit / out of
//!   `i64` range.
//! - `widen_*_to_i64` — the numeric-family source widening intermediates
//!   (`u64/i128/u128` → `i64`) used by `cast_to_integer`, with an explicit
//!   range check rather than a silent narrow-through-`i64` that would corrupt
//!   `Uint(u64::MAX)` / `Int128` / `Uint128` values outside the `i64` range.
//!
//! The exact-decimal conversion policy mirrors `runtime::value_compare`'s #212
//! comparison stance (Decimal↔integer is lossless; Decimal↔binary-float uses
//! the float's exact binary expansion via `from_f64_retain`); the cast arms
//! reuse `rust_decimal`'s own conversions rather than re-deriving it.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use selene_core::Value;

use crate::{
    SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

use super::numeric_text::normalize_signed_numeric_text;

/// Out-of-target-range overflow (`22003` numeric value out of range).
fn out_of_range(message: &'static str, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(DataExceptionSubclass::NumericValueOutOfRange, message, span)
}

/// Strict-parse failure (`22018` invalid character value for cast).
fn invalid_decimal_text(text: &str, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::InvalidCharacterValueForCast,
        format!("STRING value `{text}` is not a valid DECIMAL"),
        span,
    )
}

// ---------------------------------------------------------------------------
// → DECIMAL
// ---------------------------------------------------------------------------

/// CAST any numeric / string source to `DECIMAL` (Table 4 `EN/UN/AN/C → EN`).
///
/// Numeric sources convert exactly where representable, raising `22003` when
/// the magnitude overflows `Decimal`'s ~96-bit range (loss of a leading
/// significant digit per GR4g(i)(2)). A binary-float source uses its exact
/// binary expansion (`from_f64_retain`), matching #212's comparison policy.
/// A string source is trimmed and parsed per GR4g(ii), raising `22018` on a
/// non-`<numeric literal>` text.
pub(super) fn numeric_to_decimal(value: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    let dec = match value {
        Value::Decimal(d) => d,
        Value::Int(v) => Decimal::from(v),
        Value::Uint(v) => Decimal::from(v),
        Value::Int128(v) => Decimal::try_from_i128_with_scale(v, 0)
            .map_err(|_| out_of_range("INT128 value exceeds DECIMAL range during CAST", span))?,
        Value::Uint128(v) => {
            let signed = i128::try_from(v).map_err(|_| {
                out_of_range("UINT128 value exceeds DECIMAL range during CAST", span)
            })?;
            Decimal::try_from_i128_with_scale(signed, 0).map_err(|_| {
                out_of_range("UINT128 value exceeds DECIMAL range during CAST", span)
            })?
        }
        Value::Float(f) => float_to_decimal(f, span)?,
        Value::Float32(f) => float_to_decimal(f64::from(f), span)?,
        Value::String(s) => normalize_signed_numeric_text(s.as_str(), "DECIMAL", span)?
            .parse::<Decimal>()
            .map_err(|_| invalid_decimal_text(s.as_str(), span))?,
        // DECIMAL is signed-exact numeric (Table-4 `EN`), so a BOOLEAN source
        // is a Table-4 `N` cell exactly like BOOL→INTEGER/FLOAT — an invalid
        // type combination (22G03), not an unimplemented feature (42N01).
        // Reuse `cast`'s shared 22G03 constructor so the message + subclass stay
        // identical to the other numeric targets.
        Value::Bool(_) => {
            return Err(super::non_iso_combination(
                "CAST from BOOLEAN to a numeric type is not a valid type combination",
                span,
            ));
        }
        _ => {
            return Err(ExecutorError::FeatureNotSupportedYet {
                feature: "CAST source not supported for DECIMAL target",
                span,
            });
        }
    };
    Ok(Value::Decimal(dec))
}

fn float_to_decimal(f: f64, span: SourceSpan) -> Result<Decimal, ExecutorError> {
    if f.is_nan() {
        return Err(ExecutorError::data_exception(
            DataExceptionSubclass::InvalidCharacterValueForCast,
            "CAST of NaN to DECIMAL has no representable image",
            span,
        ));
    }
    // `from_f64_retain` returns None for ±∞ or an out-of-Decimal-range
    // magnitude — both are a loss of a leading significant digit → 22003.
    Decimal::from_f64_retain(f)
        .ok_or_else(|| out_of_range("FLOAT value exceeds DECIMAL range during CAST", span))
}

// ---------------------------------------------------------------------------
// DECIMAL → integer / string
// ---------------------------------------------------------------------------

/// CAST a `DECIMAL` source to an `i64`-canonical integer target
/// (`EN → EN`, GR4g). The fractional component truncates toward zero
/// (IA005); a magnitude that exceeds the `i64` range raises `22003`.
pub(super) fn decimal_to_int(dec: Decimal, span: SourceSpan) -> Result<Value, ExecutorError> {
    dec.trunc()
        .to_i64()
        .map(Value::Int)
        .ok_or_else(|| out_of_range("DECIMAL value exceeds INTEGER range during CAST", span))
}

/// CAST a `DECIMAL` source to a `STRING` target (`EN → C`, GR4j): the
/// canonical shortest conforming literal via `rust_decimal`'s `Display`.
pub(super) fn decimal_to_string(dec: &Decimal) -> String {
    dec.to_string()
}

// ---------------------------------------------------------------------------
// Numeric-family source widening (→ i64 integer target)
// ---------------------------------------------------------------------------

/// Widen a `u64` source to the `i64` integer target with an explicit range
/// check (`22003` when the value loses a leading significant digit — i.e. is
/// above `i64::MAX`). Never narrows through a silent `as i64`.
pub(super) fn u64_to_int(value: u64, span: SourceSpan) -> Result<Value, ExecutorError> {
    i64::try_from(value)
        .map(Value::Int)
        .map_err(|_| out_of_range("UINT value exceeds INTEGER range during CAST", span))
}

/// Widen an `i128` source to the `i64` integer target (`22003` out of range).
pub(super) fn i128_to_int(value: i128, span: SourceSpan) -> Result<Value, ExecutorError> {
    i64::try_from(value)
        .map(Value::Int)
        .map_err(|_| out_of_range("INT128 value exceeds INTEGER range during CAST", span))
}

/// Widen a `u128` source to the `i64` integer target (`22003` out of range).
pub(super) fn u128_to_int(value: u128, span: SourceSpan) -> Result<Value, ExecutorError> {
    i64::try_from(value)
        .map(Value::Int)
        .map_err(|_| out_of_range("UINT128 value exceeds INTEGER range during CAST", span))
}

#[cfg(test)]
mod tests {
    //! DECIMAL CAST arm coverage (810). One positive per numeric/string source
    //! into DECIMAL, the DECIMAL → integer/float/string directions, and the
    //! 22003/22018 error paths. These exercise the conversion helpers directly
    //! (the source variants have no GQL literal).
    use super::*;

    fn span() -> SourceSpan {
        SourceSpan::default()
    }

    fn dec(literal: &str) -> Decimal {
        literal.parse::<Decimal>().expect("valid decimal literal")
    }

    fn ok(value: Value) -> Value {
        numeric_to_decimal(value, span()).expect("→ DECIMAL succeeds")
    }

    fn status(result: Result<Value, ExecutorError>) -> String {
        result
            .expect_err("rejected")
            .gqlstatus()
            .as_str()
            .to_owned()
    }

    // --- → DECIMAL (one positive per Table-4 numeric/string source) ---

    #[test]
    fn int_to_decimal() {
        assert_eq!(ok(Value::Int(42)), Value::Decimal(dec("42")));
    }

    #[test]
    fn uint_to_decimal() {
        assert_eq!(
            ok(Value::Uint(u64::MAX)),
            Value::Decimal(dec("18446744073709551615"))
        );
    }

    #[test]
    fn int128_to_decimal() {
        assert_eq!(ok(Value::Int128(-123)), Value::Decimal(dec("-123")));
    }

    #[test]
    fn uint128_to_decimal() {
        assert_eq!(ok(Value::Uint128(123)), Value::Decimal(dec("123")));
    }

    #[test]
    fn float_to_decimal() {
        assert_eq!(ok(Value::Float(2.5)), Value::Decimal(dec("2.5")));
    }

    #[test]
    fn float32_to_decimal() {
        assert_eq!(ok(Value::Float32(2.5_f32)), Value::Decimal(dec("2.5")));
    }

    #[test]
    fn string_to_decimal() {
        assert_eq!(
            ok(Value::String(selene_core::intern("123.45").unwrap())),
            Value::Decimal(dec("123.45"))
        );
    }

    // --- → DECIMAL negatives (22003 overflow, 22018 parse/NaN) ---

    #[test]
    fn int128_overflow_to_decimal_returns_22003() {
        // i128::MAX (~1.7e38) exceeds Decimal's ~7.9e28 range.
        assert_eq!(
            status(numeric_to_decimal(Value::Int128(i128::MAX), span())),
            "22003"
        );
    }

    #[test]
    fn uint128_overflow_to_decimal_returns_22003() {
        assert_eq!(
            status(numeric_to_decimal(Value::Uint128(u128::MAX), span())),
            "22003"
        );
    }

    #[test]
    fn float_infinity_to_decimal_returns_22003() {
        assert_eq!(
            status(numeric_to_decimal(Value::Float(f64::INFINITY), span())),
            "22003"
        );
    }

    #[test]
    fn float_nan_to_decimal_returns_22018() {
        assert_eq!(
            status(numeric_to_decimal(Value::Float(f64::NAN), span())),
            "22018"
        );
    }

    #[test]
    fn string_parse_fail_to_decimal_returns_22018() {
        assert_eq!(
            status(numeric_to_decimal(
                Value::String(selene_core::intern("abc").unwrap()),
                span()
            )),
            "22018"
        );
    }

    // --- DECIMAL → integer / string ---

    #[test]
    fn decimal_to_integer_truncates_toward_zero() {
        assert_eq!(decimal_to_int(dec("3.7"), span()).unwrap(), Value::Int(3));
        assert_eq!(decimal_to_int(dec("-3.7"), span()).unwrap(), Value::Int(-3));
    }

    #[test]
    fn decimal_overflow_to_integer_returns_22003() {
        // 1e20 > i64::MAX (~9.2e18).
        assert_eq!(
            status(decimal_to_int(dec("100000000000000000000"), span())),
            "22003"
        );
    }

    #[test]
    fn decimal_to_string_canonical() {
        // `rust_decimal` preserves trailing scale in its canonical Display.
        assert_eq!(decimal_to_string(&dec("123.450")), "123.450");
    }
}
