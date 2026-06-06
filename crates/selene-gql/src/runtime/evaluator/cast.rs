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
//! - `22003` (`NumericValueOutOfRange`) — overflow on numeric→numeric,
//!   float→integer, or any widening/Decimal conversion that loses a leading
//!   significant digit (the value does not fit the target's range).
//! - `22G03` (`InvalidValueType`, datatype mismatch) — an invalid Table-4
//!   source/target combination, e.g. boolean ↔ numeric (Table 4 `N`), which
//!   ISO does not define a `CAST` for.
//! - `42N01` (`FEATURE_NOT_SUPPORTED`) — source or target outside the
//!   currently implemented explicit-cast scope (NODE / EDGE / PATH source, the
//!   temporal / bytes families, or any cast whose target is `NULL` /
//!   `NOTHING`).

use selene_core::{Record, Value};
use smallvec::SmallVec;

use crate::{
    GqlType, RecordType, SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

use super::uuid_fns::parse_uuid_string;

mod decimal;
mod float;
mod signed;
mod signed128;
mod unsigned;

use float::{FloatTarget, cast_to_float};
use signed::{SignedIntegerTarget, cast_to_signed_integer};
use signed128::cast_to_int128;
use unsigned::{UnsignedIntegerTarget, cast_to_unsigned_integer};

/// Evaluate an explicit CAST.
///
/// `value` is the already-evaluated source value; `target_type` is the
/// declared GQL target. The returned `Value` matches the canonical Rust
/// representation of `target_type` (`Integer` → `Value::Int(i64)`, `STRING`
/// → `Value::String(IStr)`, etc.). NULL propagates as NULL (ISO §22
/// universal rule). Unsupported source/target combinations produce
/// `FeatureNotSupportedYet` with a descriptive `feature` tag.
pub(super) fn eval_cast(
    value: Value,
    target_type: &GqlType,
    span: SourceSpan,
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
        return cast_to_record(value, record_type, span);
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
        GqlType::Uuid => cast_to_uuid(value, span),
        GqlType::List(element_type) => cast_to_list(value, element_type, span),
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
        _ => Err(ExecutorError::FeatureNotSupportedYet {
            feature: "CAST source not supported for BOOLEAN target",
            span,
        }),
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
        _ => {
            return Err(ExecutorError::FeatureNotSupportedYet {
                feature: "CAST source not supported for STRING target",
                span,
            });
        }
    };
    // CAST output strings construct a plain `Value::String`; the only guard is
    // the IL013 per-string byte cap (there is no global interner pool).
    match selene_core::intern(&rendered) {
        Ok(istr) => Ok(Value::String(istr)),
        Err(_err) => Err(ExecutorError::data_exception(
            DataExceptionSubclass::DataException,
            "CAST result string exceeds the maximum byte length",
            span,
        )),
    }
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

fn cast_to_list(
    value: Value,
    element_type: &GqlType,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let Value::List(items) = value else {
        return Err(ExecutorError::FeatureNotSupportedYet {
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

/// CAST a record value to a record target type per ISO/IEC 39075:2024 §20.8.
///
/// - An **open** record target (`RecordType::Open`) returns the source record unchanged
///   (GR4(e)(ii) identity).
/// - A **closed** record target re-casts each declared field. Per GR4(e)(i), for the named
///   `Value::Record` form each declared target field is resolved by name in the source:
///   a target field with no matching source field raises `22G0U` "record fields do not
///   match", and source fields not named by the target are dropped. Each resolved field's
///   value is recursively re-cast to the declared field type. (A `Value::RecordTyped` source
///   is rejected — see the fail-closed note below.)
///
/// The result is emitted as the open named `Value::Record` form, matching the §20.18
/// record-constructor output so cast results are indistinguishable from record literals
/// downstream (and validate identically against a closed-graph RECORD property).
fn cast_to_record(
    value: Value,
    target: &RecordType,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    match target {
        RecordType::Open => match value {
            Value::Record(_) | Value::RecordTyped(_) => Ok(value),
            _ => Err(non_record_source_cast(span)),
        },
        RecordType::Closed(fields) => {
            let mut out: SmallVec<[(selene_core::IStr, Value); 4]> =
                SmallVec::with_capacity(fields.len());
            match value {
                Value::Record(record) => {
                    let Record::Open(source_fields) = *record else {
                        return Err(non_record_source_cast(span));
                    };
                    for (name, ty) in fields {
                        let Some((_, source_value)) =
                            source_fields.iter().find(|(field, _)| field == name)
                        else {
                            return Err(record_fields_do_not_match(span));
                        };
                        let source_value = source_value.clone();
                        let casted = stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
                            eval_cast(source_value, ty, span)
                        })?;
                        out.push((name.clone(), casted));
                    }
                }
                // Fail-closed. SR12's closed→closed projection is by field NAME, but a
                // `Value::RecordTyped` source is catalog-bound (a `type_id` + positional
                // slots, no inline names), so name-keyed projection needs to resolve
                // `type_id` through the named-record-type catalog. That catalog is unbuilt
                // (`GraphTypeDef.record_types` is never populated) and its resolution
                // semantics are undesigned, so rather than project positionally (name-blind,
                // plausibly wrong) we reject. Unreached today: no production read path
                // materializes `Value::RecordTyped` (records surface as
                // `Value::Record(Record::Open)`, handled name-keyed above). Name-keyed
                // projection lands with the future RecordTyped read-path producer brief.
                Value::RecordTyped(_) => {
                    return Err(ExecutorError::FeatureNotSupportedYet {
                        feature: "CAST of a catalog-bound RECORD (RecordTyped) source value",
                        span,
                    });
                }
                _ => return Err(non_record_source_cast(span)),
            }
            Ok(Value::Record(Box::new(Record::Open(out))))
        }
    }
}

/// An invalid ISO §20.8 Table-4 source/target combination (a `N` cell) →
/// `22G03` datatype mismatch. Used for the boolean↔numeric cells ISO does not
/// define a `CAST` for.
fn non_iso_combination(message: &'static str, span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(DataExceptionSubclass::InvalidValueType, message, span)
}

fn non_record_source_cast(span: SourceSpan) -> ExecutorError {
    // ISO §20.8 Table 4: a non-record source to a record target is an invalid type
    // combination (`N`) → 22G03 datatype mismatch.
    ExecutorError::data_exception(
        DataExceptionSubclass::InvalidValueType,
        "CAST to RECORD requires a record source value",
        span,
    )
}

fn record_fields_do_not_match(span: SourceSpan) -> ExecutorError {
    // ISO §20.8 GR4(e): the source record fields do not match the target RECORD type.
    ExecutorError::data_exception(
        DataExceptionSubclass::RecordFieldsDoNotMatch,
        "source record fields do not match the target RECORD type",
        span,
    )
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

    #[test]
    fn recordtyped_source_to_closed_record_is_fail_closed() {
        // A catalog-bound `Value::RecordTyped` source carries no inline field names, and the
        // named-record-type catalog needed to resolve them is unbuilt — so CAST rejects it
        // (42N01 feature-not-yet-supported) rather than projecting positionally (name-blind).
        // RecordTyped is not grammar-reachable, hence this unit test.
        use selene_core::{RecordTypeId, RecordTyped};
        let value = Value::RecordTyped(Box::new(RecordTyped {
            type_id: RecordTypeId::new(1),
            values: [Some(Value::Int(1))].into_iter().collect(),
        }));
        let field = selene_core::intern("a").expect("intern field");
        let target = GqlType::Record(RecordType::Closed(vec![(field, GqlType::Integer)]));
        let err = eval_cast(value, &target, span()).expect_err("RecordTyped source rejected");
        assert!(
            matches!(err, ExecutorError::FeatureNotSupportedYet { .. }),
            "expected FeatureNotSupportedYet, got {err:?}"
        );
        assert_eq!(err.gqlstatus().as_str(), "42N01");
    }

    // -----------------------------------------------------------------------
    // 810 — DECIMAL arms + numeric-family source widening + strict-ISO bool.
    // The widened numeric source variants (Uint/Int128/Uint128/Float32/Decimal)
    // have no GQL literal, so each new cell is pinned here at the `eval_cast`
    // seam; the grammar-reachable behavior changes (bool↔string case,
    // bool→numeric reject) are additionally covered in tests/cast.rs.
    // -----------------------------------------------------------------------

    use rust_decimal::Decimal;
    use rust_decimal::prelude::FromPrimitive;

    fn cast(value: Value, target: &GqlType) -> Value {
        eval_cast(value, target, span()).expect("cast succeeds")
    }

    fn cast_status(value: Value, target: &GqlType) -> String {
        eval_cast(value, target, span())
            .expect_err("cast rejected")
            .gqlstatus()
            .as_str()
            .to_owned()
    }

    fn as_str(value: Value) -> String {
        match value {
            Value::String(istr) => istr.as_str().to_owned(),
            other => panic!("expected string, got {other:?}"),
        }
    }

    // The DECIMAL-arm cells (→ DECIMAL and DECIMAL → integer/float/string) are
    // pinned in the `cast::decimal` submodule's own `#[cfg(test)]`, where the
    // conversion helpers live. The widened-numeric + strict-bool cells below
    // route through the full `eval_cast` dispatch.

    // --- numeric-family source widening into i64 integer target ---

    #[test]
    fn uint_in_range_to_integer() {
        assert_eq!(cast(Value::Uint(7), &GqlType::Integer), Value::Int(7));
    }

    #[test]
    fn uint_above_i64_max_to_integer_returns_22003() {
        // u64::MAX must NOT silently narrow to -1; it is out of i64 range.
        assert_eq!(
            cast_status(Value::Uint(u64::MAX), &GqlType::Integer),
            "22003"
        );
    }

    #[test]
    fn int128_in_range_to_integer() {
        assert_eq!(cast(Value::Int128(-9), &GqlType::Integer), Value::Int(-9));
    }

    #[test]
    fn int128_above_i64_max_to_integer_returns_22003() {
        assert_eq!(
            cast_status(Value::Int128(i128::from(i64::MAX) + 1), &GqlType::Integer),
            "22003"
        );
    }

    #[test]
    fn uint128_in_range_to_integer() {
        assert_eq!(cast(Value::Uint128(9), &GqlType::Integer), Value::Int(9));
    }

    #[test]
    fn uint128_above_i64_max_to_integer_returns_22003() {
        assert_eq!(
            cast_status(Value::Uint128(u128::MAX), &GqlType::Integer),
            "22003"
        );
    }

    #[test]
    fn float32_to_integer_truncates() {
        assert_eq!(
            cast(Value::Float32(3.7_f32), &GqlType::Integer),
            Value::Int(3)
        );
    }

    // --- numeric-family source widening into f64 float target ---

    #[test]
    fn uint_to_float() {
        assert_eq!(cast(Value::Uint(42), &GqlType::Float), Value::Float(42.0));
    }

    #[test]
    fn int128_to_float() {
        assert_eq!(
            cast(Value::Int128(-42), &GqlType::Float),
            Value::Float(-42.0)
        );
    }

    #[test]
    fn uint128_to_float() {
        assert_eq!(
            cast(Value::Uint128(42), &GqlType::Float),
            Value::Float(42.0)
        );
    }

    #[test]
    fn float32_to_float() {
        assert_eq!(
            cast(Value::Float32(0.5_f32), &GqlType::Float),
            Value::Float(0.5)
        );
    }

    // --- numeric-family source widening into string target ---

    #[test]
    fn uint_to_string() {
        assert_eq!(as_str(cast(Value::Uint(42), &GqlType::String)), "42");
    }

    #[test]
    fn int128_to_string() {
        assert_eq!(as_str(cast(Value::Int128(-42), &GqlType::String)), "-42");
    }

    #[test]
    fn uint128_to_string() {
        assert_eq!(as_str(cast(Value::Uint128(42), &GqlType::String)), "42");
    }

    #[test]
    fn float32_to_string() {
        assert_eq!(
            as_str(cast(Value::Float32(0.5_f32), &GqlType::String)),
            "0.5"
        );
    }

    // --- strict-ISO boolean surface (3a) — every numeric source → 22G03 ---

    #[test]
    fn bool_to_integer_returns_22g03() {
        assert_eq!(cast_status(Value::Bool(true), &GqlType::Integer), "22G03");
    }

    #[test]
    fn bool_to_float_returns_22g03() {
        assert_eq!(cast_status(Value::Bool(true), &GqlType::Float), "22G03");
    }

    #[test]
    fn bool_to_decimal_returns_22g03() {
        // DECIMAL is signed-exact numeric (Table-4 `EN`); BO→EN is `N`, so a
        // boolean→DECIMAL cast is a 22G03 datatype mismatch, NOT a 42N01
        // unimplemented feature (the DECIMAL target's source fallthrough must
        // match the INTEGER/FLOAT bool arms — Codex P2 on PR #240).
        assert_eq!(cast_status(Value::Bool(true), &GqlType::Decimal), "22G03");
        assert_eq!(cast_status(Value::Bool(false), &GqlType::Decimal), "22G03");
    }

    #[test]
    fn int_to_boolean_returns_22g03() {
        assert_eq!(cast_status(Value::Int(1), &GqlType::Boolean), "22G03");
        assert_eq!(cast_status(Value::Int(0), &GqlType::Boolean), "22G03");
        assert_eq!(cast_status(Value::Int(2), &GqlType::Boolean), "22G03");
    }

    #[test]
    fn numeric_family_to_boolean_returns_22g03() {
        for value in [
            Value::Uint(1),
            Value::Int128(1),
            Value::Uint128(1),
            Value::Float(1.0),
            Value::Float32(1.0_f32),
            Value::Decimal(Decimal::from_f64(1.0).unwrap()),
        ] {
            assert_eq!(
                cast_status(value.clone(), &GqlType::Boolean),
                "22G03",
                "expected 22G03 for {value:?} -> BOOLEAN"
            );
        }
    }

    // --- bool→string uppercase (3b) + string→bool case-insensitive (3c) ---

    #[test]
    fn bool_to_string_is_uppercase() {
        assert_eq!(as_str(cast(Value::Bool(true), &GqlType::String)), "TRUE");
        assert_eq!(as_str(cast(Value::Bool(false), &GqlType::String)), "FALSE");
    }

    #[test]
    fn string_to_boolean_is_case_insensitive() {
        for text in ["true", "True", "TRUE", "tRuE", "  true  "] {
            assert_eq!(
                cast(
                    Value::String(selene_core::intern(text).unwrap()),
                    &GqlType::Boolean
                ),
                Value::Bool(true),
                "`{text}` must parse to TRUE"
            );
        }
        for text in ["false", "False", "FALSE", "fAlSe", " FALSE "] {
            assert_eq!(
                cast(
                    Value::String(selene_core::intern(text).unwrap()),
                    &GqlType::Boolean
                ),
                Value::Bool(false),
                "`{text}` must parse to FALSE"
            );
        }
    }

    #[test]
    fn string_to_boolean_garbage_still_returns_22018() {
        assert_eq!(
            cast_status(
                Value::String(selene_core::intern("yes").unwrap()),
                &GqlType::Boolean
            ),
            "22018"
        );
    }
}
