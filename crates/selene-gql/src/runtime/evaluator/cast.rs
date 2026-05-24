//! ISO/IEC 39075:2024 §22 explicit `CAST(<value> AS <target>)` dispatch matrix.
//!
//! Each `Value` x `GqlType` pair routes through this module. The matrix is
//! split into helpers per source family (numeric, string, boolean, list) so
//! the dispatch stays linear and walker-friendly. Failure modes:
//!
//! - `22018` (`InvalidCharacterValueForCast`) — strict-parse failure
//!   (string→numeric/boolean) and NaN→integer (per ISO §22, NaN has no
//!   representable integer image).
//! - `22003` (`NumericValueOutOfRange`) — overflow on numeric→numeric or
//!   float→integer that exceeds the target's range (the truncated integer
//!   would not fit).
//! - `42N01` (`FEATURE_NOT_SUPPORTED`) — source or target outside the v1.1
//!   explicit-cast scope (NODE / EDGE / PATH / RECORD source, or any cast
//!   whose target is `NULL` / `NOTHING`).

use selene_core::Value;

use crate::{
    GqlType, SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

/// Evaluate an explicit CAST.
///
/// `value` is the already-evaluated source value; `target_type` is the
/// declared GQL target. The returned `Value` matches the canonical Rust
/// representation of `target_type` (`Integer` → `Value::Int(i64)`, `STRING`
/// → `Value::String(IStr)`, etc.). NULL propagates as NULL (ISO §22
/// universal rule). Unsupported source/target combinations produce
/// `FeatureNotInV1_1` with a descriptive `feature` tag.
pub(super) fn eval_cast(
    value: Value,
    target_type: &GqlType,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    // Target-level rejections evaluated up-front.
    match target_type {
        GqlType::Null => {
            return Err(ExecutorError::FeatureNotInV1_1 {
                feature: "CAST to NULL",
                span,
            });
        }
        GqlType::Nothing => {
            return Err(ExecutorError::FeatureNotInV1_1 {
                feature: "CAST to NOTHING",
                span,
            });
        }
        _ => {}
    }

    // §22 universal: NULL casts to NULL regardless of target.
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }

    // Source-level rejections (graph-element / path / record).
    match &value {
        Value::NodeRef(_) => {
            return Err(ExecutorError::FeatureNotInV1_1 {
                feature: "CAST from NODE",
                span,
            });
        }
        Value::EdgeRef(_) => {
            return Err(ExecutorError::FeatureNotInV1_1 {
                feature: "CAST from EDGE",
                span,
            });
        }
        Value::Path(_) => {
            return Err(ExecutorError::FeatureNotInV1_1 {
                feature: "CAST from PATH",
                span,
            });
        }
        Value::Record(_) | Value::RecordTyped(_) => {
            return Err(ExecutorError::FeatureNotInV1_1 {
                feature: "CAST from RECORD",
                span,
            });
        }
        _ => {}
    }

    match target_type {
        GqlType::Integer
        | GqlType::Int64
        | GqlType::BigInt
        | GqlType::Int8
        | GqlType::Int16
        | GqlType::Int32
        | GqlType::SmallInt => cast_to_integer(value, span),
        GqlType::Float | GqlType::Float64 | GqlType::Float32 => cast_to_float(value, span),
        GqlType::Boolean => cast_to_boolean(value, span),
        GqlType::String => cast_to_string(value, span),
        GqlType::List(element_type) => cast_to_list(value, element_type, span),
        other => Err(ExecutorError::FeatureNotInV1_1 {
            feature: cast_to_type_feature(other),
            span,
        }),
    }
}

fn cast_to_integer(value: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    match value {
        Value::Int(v) => Ok(Value::Int(v)),
        Value::Bool(b) => Ok(Value::Int(i64::from(b))),
        Value::Float(f) => float_to_integer(f, span),
        Value::String(s) => string_to_integer(s.as_str(), span),
        Value::ExternalString(s) => string_to_integer(s.as_ref(), span),
        _ => Err(ExecutorError::FeatureNotInV1_1 {
            feature: "CAST source not supported for INTEGER target",
            span,
        }),
    }
}

fn cast_to_float(value: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    match value {
        Value::Float(f) => Ok(Value::Float(f)),
        Value::Int(v) => {
            // i64 → f64 is lossy for values exceeding 2^53, but ISO §22 does
            // not require lossless guarantees here. Convert directly.
            #[allow(clippy::cast_precision_loss)]
            Ok(Value::Float(v as f64))
        }
        Value::Bool(b) => Ok(Value::Float(if b { 1.0 } else { 0.0 })),
        Value::String(s) => string_to_float(s.as_str(), span),
        Value::ExternalString(s) => string_to_float(s.as_ref(), span),
        _ => Err(ExecutorError::FeatureNotInV1_1 {
            feature: "CAST source not supported for FLOAT target",
            span,
        }),
    }
}

fn cast_to_boolean(value: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    match value {
        Value::Bool(b) => Ok(Value::Bool(b)),
        Value::Int(0) => Ok(Value::Bool(false)),
        Value::Int(1) => Ok(Value::Bool(true)),
        Value::Int(_) => Err(ExecutorError::data_exception(
            DataExceptionSubclass::DataException,
            "CAST to BOOLEAN accepts only 0 or 1",
            span,
        )),
        Value::String(s) => string_to_boolean(s.as_str(), span),
        Value::ExternalString(s) => string_to_boolean(s.as_ref(), span),
        _ => Err(ExecutorError::FeatureNotInV1_1 {
            feature: "CAST source not supported for BOOLEAN target",
            span,
        }),
    }
}

fn cast_to_string(value: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    let rendered: String = match value {
        Value::Bool(b) => if b { "true" } else { "false" }.to_owned(),
        Value::Int(v) => v.to_string(),
        Value::Float(f) => format_float(f),
        Value::String(s) => s.as_str().to_owned(),
        Value::ExternalString(s) => s.as_ref().to_owned(),
        _ => {
            return Err(ExecutorError::FeatureNotInV1_1 {
                feature: "CAST source not supported for STRING target",
                span,
            });
        }
    };
    // Why: CAST output strings go to `Value::ExternalString` to avoid
    // exhausting the global IStr pool from user-controlled input. The DoS
    // guard at `tests/dos_guard.rs::no_unbudgeted_intern_call_in_selene_gql`
    // enforces this for runtime paths — per [[feedback_mem_forget_leak_dos]]
    // user-driven allocations get the unbounded path, not the bounded
    // interner.
    Ok(Value::ExternalString(rendered.into()))
}

fn cast_to_list(
    value: Value,
    element_type: &GqlType,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let Value::List(items) = value else {
        return Err(ExecutorError::FeatureNotInV1_1 {
            feature: "CAST to LIST requires a LIST source",
            span,
        });
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        // Recursive element-wise cast preserves nested-list semantics per
        // ISO §22.7. Stack-grown via `stacker::maybe_grow` to bound the
        // worst-case nested-LIST depth.
        out.push(stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
            eval_cast(item, element_type, span)
        })?);
    }
    Ok(Value::List(out))
}

fn float_to_integer(f: f64, span: SourceSpan) -> Result<Value, ExecutorError> {
    if f.is_nan() {
        return Err(ExecutorError::data_exception(
            DataExceptionSubclass::InvalidCharacterValueForCast,
            "CAST of NaN to INTEGER has no representable image",
            span,
        ));
    }
    // §22.4 — truncate toward zero (Q4). Rust `as` saturates rather than
    // wrapping, so an explicit range check enforces the 22003 contract
    // before the conversion: any |f| > i64::MAX as f64 would silently land
    // at i64::MAX/MIN otherwise.
    #[allow(clippy::cast_precision_loss)]
    let max = i64::MAX as f64;
    #[allow(clippy::cast_precision_loss)]
    let min = i64::MIN as f64;
    if !f.is_finite() || f >= max || f <= min {
        return Err(ExecutorError::data_exception(
            DataExceptionSubclass::NumericValueOutOfRange,
            "FLOAT value exceeds INTEGER range during CAST",
            span,
        ));
    }
    let truncated = f.trunc();
    #[allow(clippy::cast_possible_truncation)]
    Ok(Value::Int(truncated as i64))
}

fn string_to_integer(text: &str, span: SourceSpan) -> Result<Value, ExecutorError> {
    // Trim whitespace per ecosystem precedent (Postgres/Neo4j/SQLite); see
    // BRIEF-135a §O Q2-deviation.
    text.trim()
        .parse::<i64>()
        .map(Value::Int)
        .map_err(|_| invalid_character(text, "INTEGER", span))
}

fn string_to_float(text: &str, span: SourceSpan) -> Result<Value, ExecutorError> {
    // Trim whitespace per ecosystem precedent (Postgres/Neo4j/SQLite); see
    // BRIEF-135a §O Q2-deviation.
    text.trim()
        .parse::<f64>()
        .map(Value::Float)
        .map_err(|_| invalid_character(text, "FLOAT", span))
}

fn string_to_boolean(text: &str, span: SourceSpan) -> Result<Value, ExecutorError> {
    // ISO §22 — strict lowercase only (Q3 fold). "TRUE"/"True" are not
    // accepted; tests pin this.
    match text {
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        _ => Err(invalid_character(text, "BOOLEAN", span)),
    }
}

fn invalid_character(text: &str, target: &str, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::InvalidCharacterValueForCast,
        format!("STRING value `{text}` is not a valid {target}"),
        span,
    )
}

fn format_float(f: f64) -> String {
    if f.is_nan() {
        "NaN".to_owned()
    } else if f.is_infinite() {
        if f > 0.0 { "Infinity" } else { "-Infinity" }.to_owned()
    } else {
        format!("{f}")
    }
}

fn cast_to_type_feature(target: &GqlType) -> &'static str {
    match target {
        GqlType::Decimal => "CAST to DECIMAL",
        GqlType::Bytes | GqlType::Binary | GqlType::VarBinary => "CAST to BYTES",
        GqlType::ZonedDateTime => "CAST to ZONED DATETIME",
        GqlType::LocalDateTime => "CAST to LOCAL DATETIME",
        GqlType::Date => "CAST to DATE",
        GqlType::ZonedTime => "CAST to ZONED TIME",
        GqlType::LocalTime => "CAST to LOCAL TIME",
        GqlType::Duration => "CAST to DURATION",
        GqlType::Record(_) => "CAST to RECORD",
        GqlType::Path => "CAST to PATH",
        GqlType::GraphRef => "CAST to GRAPH",
        GqlType::NodeRef => "CAST to NODE",
        GqlType::EdgeRef => "CAST to EDGE",
        GqlType::TableRef => "CAST to TABLE",
        GqlType::Int128 | GqlType::Uint128 => "CAST to 128-bit integer",
        GqlType::Uint8 | GqlType::Uint16 | GqlType::Uint32 | GqlType::Uint64 => {
            "CAST to unsigned integer"
        }
        _ => "CAST to unsupported target type",
    }
}

#[cfg(test)]
mod tests {
    //! Unit coverage for runtime CAST branches that are not addressable
    //! through GQL grammar. The integration suite at
    //! `crates/selene-gql/tests/cast.rs` covers every grammar-reachable
    //! path; these tests close the remaining BF-class gaps (NaN, overflow,
    //! ±Infinity) where the grammar has no literal for the input value.
    use super::*;
    use crate::SourceSpan;
    use crate::runtime::ExecutorError;

    fn span() -> SourceSpan {
        SourceSpan::default()
    }

    #[test]
    fn float_nan_to_integer_returns_22018() {
        let err = eval_cast(Value::Float(f64::NAN), &GqlType::Integer, span())
            .expect_err("NaN cast is rejected");
        let ExecutorError::DataException { subclass, .. } = err else {
            panic!("expected DataException, got {err:?}");
        };
        assert_eq!(
            subclass,
            DataExceptionSubclass::InvalidCharacterValueForCast
        );
    }

    #[test]
    fn float_overflow_to_integer_returns_22003() {
        let err = eval_cast(Value::Float(1e30_f64), &GqlType::Integer, span())
            .expect_err("overflow cast is rejected");
        let ExecutorError::DataException { subclass, .. } = err else {
            panic!("expected DataException, got {err:?}");
        };
        assert_eq!(subclass, DataExceptionSubclass::NumericValueOutOfRange);
    }

    #[test]
    fn float_negative_overflow_to_integer_returns_22003() {
        let err = eval_cast(Value::Float(-1e30_f64), &GqlType::Integer, span())
            .expect_err("negative overflow cast is rejected");
        let ExecutorError::DataException { subclass, .. } = err else {
            panic!("expected DataException, got {err:?}");
        };
        assert_eq!(subclass, DataExceptionSubclass::NumericValueOutOfRange);
    }

    #[test]
    fn float_positive_infinity_to_integer_returns_22003() {
        let err = eval_cast(Value::Float(f64::INFINITY), &GqlType::Integer, span())
            .expect_err("+inf cast is rejected");
        let ExecutorError::DataException { subclass, .. } = err else {
            panic!("expected DataException, got {err:?}");
        };
        assert_eq!(subclass, DataExceptionSubclass::NumericValueOutOfRange);
    }

    #[test]
    fn float_negative_infinity_to_integer_returns_22003() {
        let err = eval_cast(Value::Float(f64::NEG_INFINITY), &GqlType::Integer, span())
            .expect_err("-inf cast is rejected");
        let ExecutorError::DataException { subclass, .. } = err else {
            panic!("expected DataException, got {err:?}");
        };
        assert_eq!(subclass, DataExceptionSubclass::NumericValueOutOfRange);
    }
}
