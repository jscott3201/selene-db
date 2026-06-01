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
//! - `42N01` (`FEATURE_NOT_SUPPORTED`) — source or target outside the
//!   currently implemented explicit-cast scope (NODE / EDGE / PATH / RECORD
//!   source, or any cast whose target is `NULL` / `NOTHING`).

use selene_core::{Record, Value};
use smallvec::SmallVec;

use crate::{
    GqlType, RecordType, SourceSpan,
    runtime::{DataExceptionSubclass, ExecutorError},
};

use super::uuid_fns::parse_uuid_string;

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
    // Target-level rejections evaluated up-front.
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

    // §22 universal: NULL casts to NULL regardless of target.
    if matches!(value, Value::Null) {
        return Ok(Value::Null);
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
        GqlType::Uuid => cast_to_uuid(value, span),
        GqlType::List(element_type) => cast_to_list(value, element_type, span),
        other => Err(ExecutorError::FeatureNotSupportedYet {
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
        _ => Err(ExecutorError::FeatureNotSupportedYet {
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
        _ => Err(ExecutorError::FeatureNotSupportedYet {
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
        _ => Err(ExecutorError::FeatureNotSupportedYet {
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
        GqlType::Bytes => "CAST to BYTES",
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
}
