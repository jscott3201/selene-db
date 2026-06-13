//! Typed-predicate target support checks.

use crate::{GqlType, RecordType};

pub(super) fn is_supported_typed_target(ty: &GqlType) -> bool {
    match ty {
        GqlType::NotNull(inner) => is_supported_typed_target(inner),
        GqlType::Any
        | GqlType::AnyProperty
        | GqlType::String
        | GqlType::CharacterString(_)
        | GqlType::Boolean
        | GqlType::Integer
        | GqlType::Float
        | GqlType::Int8
        | GqlType::Int16
        | GqlType::Int32
        | GqlType::Int64
        | GqlType::Int128
        | GqlType::Uint8
        | GqlType::Uint16
        | GqlType::Uint32
        | GqlType::Uint64
        | GqlType::Uint128
        | GqlType::USmallInt
        | GqlType::Uint
        | GqlType::UBigInt
        | GqlType::SmallInt
        | GqlType::BigInt
        | GqlType::Decimal
        | GqlType::DecimalExact(_)
        | GqlType::Float32
        | GqlType::Float64
        | GqlType::Real
        | GqlType::Double
        | GqlType::Bytes
        | GqlType::ByteString(_)
        | GqlType::Uuid
        | GqlType::Json
        | GqlType::ZonedDateTime
        | GqlType::LocalDateTime
        | GqlType::Date
        | GqlType::ZonedTime
        | GqlType::LocalTime
        | GqlType::Duration
        | GqlType::DurationYearToMonth
        | GqlType::DurationDayToSecond
        | GqlType::Vector
        | GqlType::Path
        | GqlType::Null
        | GqlType::Nothing => true,
        GqlType::List(inner)
        | GqlType::BoundedList {
            element_type: inner,
            ..
        } => is_supported_typed_target(inner),
        GqlType::ClosedDynamicUnion(components) => components.iter().all(is_supported_typed_target),
        GqlType::Record(RecordType::Open) => true,
        GqlType::Record(RecordType::Closed(fields)) => {
            fields.iter().all(|(_, ty)| is_supported_typed_target(ty))
        }
        GqlType::NodeRef | GqlType::EdgeRef => true,
        GqlType::GraphRef | GqlType::TableRef(_) => false,
    }
}
