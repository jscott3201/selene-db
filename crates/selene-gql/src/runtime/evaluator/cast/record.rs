use selene_core::{Record, Value};
use smallvec::SmallVec;

use crate::{
    RecordType, SourceSpan,
    runtime::{DataExceptionSubclass, EvalCtx, ExecutorError},
};

/// CAST a record value to a record target type per ISO/IEC 39075:2024 §20.8.
///
/// Open record targets return the source record unchanged. Closed record targets
/// resolve each declared target field by name, cast it recursively, and drop
/// source fields not named by the target.
pub(super) fn cast_to_record(
    value: Value,
    target: &RecordType,
    span: SourceSpan,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    match target {
        RecordType::Open => match value {
            Value::Record(_) | Value::RecordTyped(_) => Ok(value),
            _ => Err(non_record_source_cast(span)),
        },
        RecordType::Closed(fields) => {
            let mut out: SmallVec<[(selene_core::DbString, Value); 4]> =
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
                            super::eval_cast(source_value, ty, span, ctx)
                        })?;
                        out.push((name.clone(), casted));
                    }
                }
                // Fail-closed. SR12's closed-to-closed projection is by field name, but a
                // `Value::RecordTyped` source is catalog-bound and currently carries no inline
                // names. Keep this rejected until the named-record-type catalog is wired.
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
    // ISO §20.8 Table 4: a non-record source to a record target is an invalid
    // type combination, not a missing feature.
    ExecutorError::data_exception(
        DataExceptionSubclass::InvalidValueType,
        "CAST to RECORD requires a record source value",
        span,
    )
}

fn record_fields_do_not_match(span: SourceSpan) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::RecordFieldsDoNotMatch,
        "source record fields do not match the target RECORD type",
        span,
    )
}
