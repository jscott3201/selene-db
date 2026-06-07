//! ISO/IEC 39075:2024 §20.8 explicit `CAST(<value> AS <target>)` dispatch matrix.
//!
//! Each `Value` x `GqlType` pair routes through this module. The matrix is
//! split into helpers per source family (numeric, string, boolean, list,
//! decimal) so the dispatch stays linear and walker-friendly. The numeric
//! family is `Table 4`'s signed-/unsigned-exact + approximate base types
//! (`EN`/`UN`/`AN`); every `EN/UN/AN ↔ EN/UN/AN/C` cell is mandated `Y`, so a
//! `Uint`/`Int128`/`Uint128`/`Float32`/`Decimal` source widens to its target
//! the same as `Int`/`Float`. Failure modes:
//!
//! - `22018` (`InvalidCharacterValueForCast`) — strict-parse failure
//!   (string→numeric/boolean/decimal) and NaN→integer/decimal (per ISO §20.8,
//!   NaN has no representable exact image).
//! - `22007` (`InvalidDatetimeFormat`) — strict-parse failure for
//!   string→date/time/datetime casts.
//! - `22G0H` (`InvalidDurationFormat`) — strict-parse failure for
//!   string→duration casts.
//! - `22003` (`NumericValueOutOfRange`) — overflow on numeric→numeric,
//!   float→integer, or any widening/Decimal conversion that loses a leading
//!   significant digit (the value does not fit the target's range).
//! - `22G03` (`InvalidValueType`, datatype mismatch) — an invalid Table-4
//!   source/target combination, e.g. boolean ↔ numeric (Table 4 `N`), which
//!   ISO does not define a `CAST` for.
//! - `42N01` (`FEATURE_NOT_SUPPORTED`) — source or target outside the
//!   currently implemented explicit-cast scope (NODE / EDGE / PATH source or
//!   any cast whose target is `NULL` / `NOTHING`).

use selene_core::{DbString, JsonValue, Value};

use crate::{
    GqlType, SourceSpan,
    runtime::{DataExceptionSubclass, EvalCtx, ExecutorError},
};

use super::uuid_fns::parse_uuid_string;

mod decimal;
mod float;
mod numeric_text;
mod record;
mod signed;
mod signed128;
mod temporal;
mod unsigned;
mod vector;

use float::{FloatTarget, cast_to_float};
use record::cast_to_record;
use signed::{SignedIntegerTarget, cast_to_signed_integer};
use signed128::cast_to_int128;
use temporal::cast_to_temporal;
use unsigned::{UnsignedIntegerTarget, cast_to_unsigned_integer};
use vector::cast_to_vector;

/// Evaluate an explicit CAST.
///
/// `value` is the already-evaluated source value; `target_type` is the
/// declared GQL target. The returned `Value` matches the canonical Rust
/// representation of `target_type` (`Integer` → `Value::Int(i64)`, `STRING`
/// → `Value::String(DbString)`, etc.). NULL propagates as NULL (ISO §22
/// universal rule). Unsupported source/target combinations produce
/// `FeatureNotSupportedYet` with a descriptive `feature` tag.
pub(super) fn eval_cast(
    value: Value,
    target_type: &GqlType,
    span: SourceSpan,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    // §22 universal: NULL casts to NULL regardless of target.
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
    }

    // Target-level rejections for non-NULL source values.
    match target_type {
        GqlType::Null => {
            return Err(ExecutorError::FeatureNotSupportedYet {
                feature: "CAST to NULL",
                span,
            });
        }
        GqlType::Nothing => {
            return Err(ExecutorError::FeatureNotSupportedYet {
                feature: "CAST to NOTHING",
                span,
            });
        }
        _ => {}
    }

    // A RECORD target is handled before the generic source-rejection block: per ISO §20.8
    // Table 4 the only valid source for a record target is a record (R -> R), so a record
    // source reaching a record target must not be spuriously rejected below.
    if let GqlType::Record(record_type) = target_type {
        return cast_to_record(value, record_type, span, ctx);
    }

    // Source-level rejections (graph-element / path / record).
    match &value {
        Value::NodeRef(_) => {
            return Err(ExecutorError::FeatureNotSupportedYet {
                feature: "CAST from NODE",
                span,
            });
        }
        Value::EdgeRef(_) => {
            return Err(ExecutorError::FeatureNotSupportedYet {
                feature: "CAST from EDGE",
                span,
            });
        }
        Value::Path(_) => {
            return Err(ExecutorError::FeatureNotSupportedYet {
                feature: "CAST from PATH",
                span,
            });
        }
        Value::Record(_) | Value::RecordTyped(_) => {
            // A record source to a non-record (scalar/list) target is an invalid type
            // combination per ISO §20.8 Table 4 (`N`), i.e. a 22G03 datatype mismatch —
            // not a missing feature.
            return Err(ExecutorError::data_exception(
                DataExceptionSubclass::InvalidValueType,
                "CAST from RECORD to a non-record type is not a valid type combination",
                span,
            ));
        }
        Value::Bytes(_) if !matches!(target_type, GqlType::Bytes) => {
            // ISO §20.8 Table 4: byte strings only cast to byte strings. Every
            // byte-string source to a non-BYTES target is an invalid
            // source/target combination, not an unimplemented conversion.
            return Err(non_iso_combination(
                "CAST from BYTES to a non-BYTES type is not a valid type combination",
                span,
            ));
        }
        Value::Json(_) if !matches!(target_type, GqlType::Json | GqlType::String) => {
            return Err(non_iso_combination(
                "CAST from JSON to this target is not a valid type combination",
                span,
            ));
        }
        _ => {}
    }

    match target_type {
        GqlType::Integer | GqlType::Int64 | GqlType::BigInt => {
            cast_to_signed_integer(value, SignedIntegerTarget::I64, span)
        }
        GqlType::Int8 => cast_to_signed_integer(value, SignedIntegerTarget::I8, span),
        GqlType::Int16 | GqlType::SmallInt => {
            cast_to_signed_integer(value, SignedIntegerTarget::I16, span)
        }
        GqlType::Int32 => cast_to_signed_integer(value, SignedIntegerTarget::I32, span),
        GqlType::Int128 => cast_to_int128(value, span),
        GqlType::Uint8 => cast_to_unsigned_integer(value, UnsignedIntegerTarget::U8, span),
        GqlType::Uint16 => cast_to_unsigned_integer(value, UnsignedIntegerTarget::U16, span),
        GqlType::Uint32 => cast_to_unsigned_integer(value, UnsignedIntegerTarget::U32, span),
        GqlType::Uint64 => cast_to_unsigned_integer(value, UnsignedIntegerTarget::U64, span),
        GqlType::Uint128 => cast_to_unsigned_integer(value, UnsignedIntegerTarget::U128, span),
        GqlType::Float | GqlType::Float64 => cast_to_float(value, FloatTarget::F64, span),
        GqlType::Float32 => cast_to_float(value, FloatTarget::F32, span),
        GqlType::Decimal => decimal::numeric_to_decimal(value, span),
        GqlType::Boolean => cast_to_boolean(value, span),
        GqlType::String => cast_to_string(value, span),
        GqlType::Bytes => cast_to_bytes(value, span),
        GqlType::Uuid => cast_to_uuid(value, span),
        GqlType::Json => cast_to_json(value, span),
        GqlType::Vector => cast_to_vector(value, span),
        GqlType::ZonedDateTime
        | GqlType::LocalDateTime
        | GqlType::Date
        | GqlType::ZonedTime
        | GqlType::LocalTime
        | GqlType::Duration => cast_to_temporal(value, target_type, span, ctx),
        GqlType::List(element_type) => cast_to_list(value, element_type, span, ctx),
        other => Err(ExecutorError::FeatureNotSupportedYet {
            feature: cast_to_type_feature(other),
            span,
        }),
    }
}

fn cast_to_boolean(value: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    // Per ISO §20.8 Table 4 the only valid sources for a boolean target are
    // `BO` (identity, GR4 boolean-source rule) and `C` (string, GR4q). Every
    // numeric source (`EN`/`UN`/`AN`, including DECIMAL) is a `N` cell — ISO
    // has no numeric→boolean cast — so it is a 22G03 datatype mismatch, not a
    // 0/1-truthiness extension.
    match value {
        Value::Bool(b) => Ok(Value::Bool(b)),
        Value::String(s) => string_to_boolean(s.as_str(), span),
        Value::Int(_)
        | Value::Uint(_)
        | Value::Int128(_)
        | Value::Uint128(_)
        | Value::Float(_)
        | Value::Float32(_)
        | Value::Decimal(_) => Err(non_iso_combination(
            "CAST from a numeric type to BOOLEAN is not a valid type combination",
            span,
        )),
        other => Err(
            non_iso_static_source_for_target(&other, "BOOLEAN", span).unwrap_or(
                ExecutorError::FeatureNotSupportedYet {
                    feature: "CAST source not supported for BOOLEAN target",
                    span,
                },
            ),
        ),
    }
}

fn cast_to_string(value: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    let rendered: String = match value {
        // ISO §20.8 GR4(j)(v)(1): boolean→string renders the UPPERCASE literal
        // `'TRUE'`/`'FALSE'` (GR4v), not lowercase.
        Value::Bool(b) => if b { "TRUE" } else { "FALSE" }.to_owned(),
        // Numeric → C (GR4j): the shortest conforming literal. Every numeric
        // family is a Table-4 `Y` source, rendered through its own `Display`.
        Value::Int(v) => v.to_string(),
        Value::Uint(v) => v.to_string(),
        Value::Int128(v) => v.to_string(),
        Value::Uint128(v) => v.to_string(),
        Value::Float(f) => format_float(f),
        Value::Float32(f) => format_float(f64::from(f)),
        Value::Decimal(d) => decimal::decimal_to_string(&d),
        Value::String(s) => s.as_str().to_owned(),
        Value::Uuid(v) => v.to_string(),
        Value::ZonedDateTime(v) => format!("{}{}", v.datetime(), v.offset()),
        Value::LocalDateTime(v) => v.to_string(),
        Value::Date(v) => v.to_string(),
        Value::ZonedTime(v) => format!("{}{}", v.time(), v.offset()),
        Value::LocalTime(v) => v.to_string(),
        Value::Duration(v) => v.to_string(),
        Value::Json(v) => v.to_canonical_string(),
        other => {
            if let Some(error) = non_iso_static_source_for_target(&other, "STRING", span) {
                return Err(error);
            }
            return Err(ExecutorError::FeatureNotSupportedYet {
                feature: "CAST source not supported for STRING target",
                span,
            });
        }
    };
    // CAST output strings construct a plain `Value::String`; the only guard is
    // the IL013 per-string byte cap (there is no global string pool).
    match DbString::from_string(rendered) {
        Ok(db_string) => Ok(Value::String(db_string)),
        Err(_err) => Err(ExecutorError::data_exception(
            DataExceptionSubclass::DataException,
            "CAST result string exceeds the maximum byte length",
            span,
        )),
    }
}

fn cast_to_json(value: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    match value {
        Value::Json(value) => Ok(Value::Json(value)),
        Value::String(value) => parse_json_value(value.as_str(), span),
        _ => Err(non_iso_combination(
            "CAST from this source to JSON is not a valid type combination",
            span,
        )),
    }
}

pub(super) fn parse_json_value(text: &str, span: SourceSpan) -> Result<Value, ExecutorError> {
    JsonValue::parse_str(text).map(Value::Json).map_err(|err| {
        if err.gqlstatus() == "22018" {
            ExecutorError::data_exception(
                DataExceptionSubclass::InvalidCharacterValueForCast,
                format!("STRING value is not valid JSON: {err}"),
                span,
            )
        } else {
            ExecutorError::data_exception(
                DataExceptionSubclass::DataException,
                format!("JSON value exceeds implementation-defined limits: {err}"),
                span,
            )
        }
    })
}

fn cast_to_uuid(value: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    match value {
        Value::Uuid(v) => Ok(Value::Uuid(v)),
        Value::String(s) => parse_uuid_string(s.as_str(), span).map(Value::Uuid),
        _ => Err(ExecutorError::FeatureNotSupportedYet {
            feature: "CAST source not supported for UUID target",
            span,
        }),
    }
}

fn cast_to_bytes(value: Value, span: SourceSpan) -> Result<Value, ExecutorError> {
    match value {
        Value::Bytes(value) => Ok(Value::Bytes(value)),
        _ => Err(non_iso_combination(
            "CAST from a non-BYTES type to BYTES is not a valid type combination",
            span,
        )),
    }
}

fn cast_to_list(
    value: Value,
    element_type: &GqlType,
    span: SourceSpan,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let items = match value {
        Value::List(items) => items,
        other => {
            return Err(
                non_iso_static_source_for_target(&other, "LIST", span).unwrap_or(
                    ExecutorError::FeatureNotSupportedYet {
                        feature: "CAST to LIST requires a LIST source",
                        span,
                    },
                ),
            );
        }
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        // Recursive element-wise cast preserves nested-list semantics per
        // ISO §22.7. Stack-grown via `stacker::maybe_grow` to bound the
        // worst-case nested-LIST depth.
        out.push(stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
            eval_cast(item, element_type, span, ctx)
        })?);
    }
    Ok(Value::List(out))
}

/// An invalid ISO §20.8 Table-4 source/target combination (a `N` cell) →
/// `22G03` datatype mismatch. Used for the boolean↔numeric cells ISO does not
/// define a `CAST` for.
fn non_iso_combination(message: impl Into<String>, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(DataExceptionSubclass::InvalidValueType, message, span)
}

pub(super) fn non_iso_static_source_for_target(
    value: &Value,
    target: &'static str,
    span: SourceSpan,
) -> Option<ExecutorError> {
    let source = iso_static_source_name(value)?;
    Some(non_iso_combination(
        format!("CAST from {source} to {target} is not a valid type combination"),
        span,
    ))
}

fn iso_static_source_name(value: &Value) -> Option<&'static str> {
    Some(match value {
        Value::Bool(_) => "BOOLEAN",
        Value::Int(_) | Value::Int128(_) | Value::Decimal(_) => "signed exact numeric",
        Value::Uint(_) | Value::Uint128(_) => "unsigned exact numeric",
        Value::Float(_) | Value::Float32(_) => "approximate numeric",
        Value::String(_) => "STRING",
        Value::Bytes(_) => "BYTES",
        Value::List(_) => "LIST",
        Value::Record(_) | Value::RecordTyped(_) => "RECORD",
        Value::Path(_) => "PATH",
        Value::ZonedDateTime(_) | Value::LocalDateTime(_) | Value::Date(_) => "datetime",
        Value::ZonedTime(_) | Value::LocalTime(_) => "time",
        Value::Duration(_) => "DURATION",
        Value::Null => "NULL",
        Value::NodeRef(_) | Value::EdgeRef(_) | Value::GraphRef(_) | Value::TableRef(_) => {
            return None;
        }
        Value::Extended { .. } | Value::Uuid(_) | Value::Vector(_) | Value::Json(_) => return None,
        _ => return None,
    })
}

fn string_to_boolean(text: &str, span: SourceSpan) -> Result<Value, ExecutorError> {
    // ISO §20.8 GR4(q) defers C→BO to the §21.2 boolean-literal rules, which
    // are case-insensitive (`TRUE`/`True`/`true`, `FALSE`/`False`/`false`).
    // Trim leading/trailing whitespace consistent with the numeric GR4(g)(ii)
    // truncating-whitespace rule used by string-to-integer casts.
    match text.trim().to_ascii_lowercase().as_str() {
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
        GqlType::Bytes => "CAST to BYTES",
        GqlType::ZonedDateTime => "CAST to ZONED DATETIME",
        GqlType::LocalDateTime => "CAST to LOCAL DATETIME",
        GqlType::Date => "CAST to DATE",
        GqlType::ZonedTime => "CAST to ZONED TIME",
        GqlType::LocalTime => "CAST to LOCAL TIME",
        GqlType::Duration => "CAST to DURATION",
        GqlType::Vector => "CAST to VECTOR",
        GqlType::Json => "CAST to JSON",
        GqlType::Record(_) => "CAST to RECORD",
        GqlType::Path => "CAST to PATH",
        GqlType::GraphRef => "CAST to GRAPH",
        GqlType::NodeRef => "CAST to NODE",
        GqlType::EdgeRef => "CAST to EDGE",
        GqlType::TableRef => "CAST to TABLE",
        _ => "CAST to unsupported target type",
    }
}

#[cfg(test)]
mod tests;
