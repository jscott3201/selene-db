//! Typed-index schema serialization helpers.

use selene_core::SchemaPropertyIndexKind;

use crate::typed_index::TypedIndexKind;

/// Convert a live typed-index kind to its durable schema-change kind.
pub(crate) const fn schema_kind_from(kind: TypedIndexKind) -> SchemaPropertyIndexKind {
    match kind {
        TypedIndexKind::Bool => SchemaPropertyIndexKind::Bool,
        TypedIndexKind::I64 => SchemaPropertyIndexKind::I64,
        TypedIndexKind::U64 => SchemaPropertyIndexKind::U64,
        TypedIndexKind::I128 => SchemaPropertyIndexKind::I128,
        TypedIndexKind::U128 => SchemaPropertyIndexKind::U128,
        TypedIndexKind::Decimal => SchemaPropertyIndexKind::Decimal,
        TypedIndexKind::F32 => SchemaPropertyIndexKind::F32,
        TypedIndexKind::F64 => SchemaPropertyIndexKind::F64,
        TypedIndexKind::String => SchemaPropertyIndexKind::String,
        TypedIndexKind::Date => SchemaPropertyIndexKind::Date,
        TypedIndexKind::LocalDateTime => SchemaPropertyIndexKind::LocalDateTime,
        TypedIndexKind::ZonedDateTime => SchemaPropertyIndexKind::ZonedDateTime,
        TypedIndexKind::LocalTime => SchemaPropertyIndexKind::LocalTime,
        TypedIndexKind::ZonedTime => SchemaPropertyIndexKind::ZonedTime,
        TypedIndexKind::Uuid => SchemaPropertyIndexKind::Uuid,
    }
}
