//! Structural "does this value satisfy this type?" matcher, shared by the typed-parameter
//! validator ([`crate::runtime::parameter_type`]) and the `IS [NOT] TYPED` predicate
//! evaluator ([`crate::runtime::evaluator::predicates`]).
//!
//! The matcher works directly on the AST [`GqlType`]/[`RecordType`] — the expression layer
//! has no catalog, so closed-record conformance is decided structurally here rather than via
//! the rkyv-side catalog descriptor. For a closed record this is a **membership** test with
//! field-name-set equality (ISO 39075:2024 §4.15.4): no missing and no extra fields. (This is
//! deliberately stricter than the `CAST` subset/projection rule in
//! [`crate::runtime::evaluator::cast`].)

use selene_core::{DurationTypeQualifier, Record, Value};

use crate::{GqlType, RecordType};

/// Return true when `value` structurally conforms to the AST type `ty`.
///
/// This is a two-valued (not three-valued) test: a `Value::Null` operand conforms only to
/// `GqlType::Null`, so `NULL IS TYPED <T>` is `false` for any material `T`.
pub(crate) fn value_matches_gql_type(value: &Value, ty: &GqlType) -> bool {
    match ty {
        GqlType::String => matches!(value, Value::String(_)),
        GqlType::Uuid => matches!(value, Value::Uuid(_)),
        GqlType::Json => matches!(value, Value::Json(_)),
        GqlType::Boolean => matches!(value, Value::Bool(_)),
        GqlType::Integer | GqlType::Int64 | GqlType::BigInt => matches!(value, Value::Int(_)),
        GqlType::Int8 => matches!(value, Value::Int(value) if i8::try_from(*value).is_ok()),
        GqlType::Int16 | GqlType::SmallInt => {
            matches!(value, Value::Int(value) if i16::try_from(*value).is_ok())
        }
        GqlType::Int32 => matches!(value, Value::Int(value) if i32::try_from(*value).is_ok()),
        GqlType::Int128 => matches!(value, Value::Int128(_)),
        GqlType::Uint8 => matches!(value, Value::Uint(value) if u8::try_from(*value).is_ok()),
        GqlType::Uint16 => matches!(value, Value::Uint(value) if u16::try_from(*value).is_ok()),
        GqlType::Uint32 => matches!(value, Value::Uint(value) if u32::try_from(*value).is_ok()),
        GqlType::Uint64 => matches!(value, Value::Uint(_)),
        GqlType::USmallInt => matches!(value, Value::Uint(value) if u16::try_from(*value).is_ok()),
        GqlType::Uint => matches!(value, Value::Uint(value) if u32::try_from(*value).is_ok()),
        GqlType::UBigInt => matches!(value, Value::Uint(_)),
        GqlType::Uint128 => matches!(value, Value::Uint128(_)),
        GqlType::Float => matches!(value, Value::Float(_) | Value::Float32(_)),
        GqlType::Float32 | GqlType::Real => matches!(value, Value::Float32(_)),
        GqlType::Float64 | GqlType::Double => matches!(value, Value::Float(_)),
        GqlType::Decimal => matches!(value, Value::Decimal(_)),
        GqlType::Bytes => matches!(value, Value::Bytes(_)),
        GqlType::ByteString(byte_type) => match value {
            Value::Bytes(value) => {
                let len = value.len() as u64;
                len >= byte_type.min_len && len <= byte_type.max_len
            }
            _ => false,
        },
        GqlType::ZonedDateTime => matches!(value, Value::ZonedDateTime(_)),
        GqlType::LocalDateTime => matches!(value, Value::LocalDateTime(_)),
        GqlType::Date => matches!(value, Value::Date(_)),
        GqlType::ZonedTime => matches!(value, Value::ZonedTime(_)),
        GqlType::LocalTime => matches!(value, Value::LocalTime(_)),
        GqlType::Duration => matches!(value, Value::Duration(_)),
        GqlType::DurationYearToMonth => {
            matches!(value, Value::Duration(span) if DurationTypeQualifier::YearToMonth.matches_span(span))
        }
        GqlType::DurationDayToSecond => {
            matches!(value, Value::Duration(span) if DurationTypeQualifier::DayToSecond.matches_span(span))
        }
        GqlType::Vector => matches!(value, Value::Vector(_)),
        GqlType::Record(record) => value_matches_record_type(value, record),
        GqlType::List(inner) => match value {
            Value::List(values) => values
                .iter()
                .all(|value| value_matches_gql_type(value, inner)),
            _ => false,
        },
        GqlType::Path => matches!(value, Value::Path(_)),
        GqlType::GraphRef => matches!(value, Value::GraphRef(_)),
        GqlType::NodeRef => matches!(value, Value::NodeRef(_)),
        GqlType::EdgeRef => matches!(value, Value::EdgeRef(_)),
        GqlType::TableRef => matches!(value, Value::TableRef(_)),
        GqlType::Null => matches!(value, Value::Null),
        GqlType::Nothing => false,
    }
}

fn value_matches_record_type(value: &Value, record: &RecordType) -> bool {
    match record {
        RecordType::Open => matches!(value, Value::Record(_) | Value::RecordTyped(_)),
        RecordType::Closed(fields) => match value {
            Value::Record(record) => match record.as_ref() {
                Record::Open(values) => {
                    values.len() == fields.len()
                        && fields.iter().all(|(name, ty)| {
                            values
                                .iter()
                                .find(|(field, _)| field == name)
                                .is_some_and(|(_, value)| value_matches_gql_type(value, ty))
                        })
                }
                _ => false,
            },
            // Fail-closed. A `Value::RecordTyped` is catalog-bound — it carries a `type_id`
            // plus positional slots, no inline field names — so verifying §4.15.4
            // name-set equality against the user-written closed type requires resolving
            // `type_id` through the named-record-type catalog. That catalog is unbuilt
            // (`GraphTypeDef.record_types` is never populated) and its resolution semantics
            // are undesigned, so rather than match positionally (name-blind, plausibly
            // wrong) we conservatively report non-conformance. Unreached today: no
            // production read path materializes `Value::RecordTyped` (records surface as
            // `Value::Record(Record::Open)`, handled name-keyed above). Name-keyed
            // conformance lands with the future RecordTyped read-path producer brief.
            Value::RecordTyped(_) => false,
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::value_matches_gql_type;
    use crate::{GqlType, RecordType};
    use selene_core::{RecordTypeId, RecordTyped, Value};

    fn sample_recordtyped() -> Value {
        Value::RecordTyped(Box::new(RecordTyped {
            type_id: RecordTypeId::new(1),
            values: [Some(Value::Int(1))].into_iter().collect(),
        }))
    }

    #[test]
    fn closed_record_type_rejects_recordtyped_operand_fail_closed() {
        // A catalog-bound RecordTyped cannot be name-verified without the (unbuilt)
        // named-record-type catalog, so a closed record type conservatively does not match.
        let field = selene_core::db_string("a").expect("db_string field");
        let ty = GqlType::Record(RecordType::Closed(vec![(field, GqlType::Integer)]));
        assert!(!value_matches_gql_type(&sample_recordtyped(), &ty));
    }

    #[test]
    fn open_record_type_accepts_recordtyped_operand() {
        // The open record type needs no field names, so any record value conforms.
        let ty = GqlType::Record(RecordType::Open);
        assert!(value_matches_gql_type(&sample_recordtyped(), &ty));
    }
}
