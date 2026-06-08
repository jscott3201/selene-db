//! Typed-index schema serialization helpers.

use selene_core::SchemaPropertyIndexKind;

use crate::typed_index::TypedIndexKind;

/// Convert a live typed-index kind to its durable schema-change kind.
pub(crate) const fn schema_kind_from(kind: TypedIndexKind) -> SchemaPropertyIndexKind {
    match kind {
        TypedIndexKind::Bool => SchemaPropertyIndexKind::Bool,
        TypedIndexKind::I64 => SchemaPropertyIndexKind::I64,
        TypedIndexKind::U64 => SchemaPropertyIndexKind::U64,
        TypedIndexKind::F64 => SchemaPropertyIndexKind::F64,
        TypedIndexKind::String => SchemaPropertyIndexKind::String,
        TypedIndexKind::Date => SchemaPropertyIndexKind::Date,
        TypedIndexKind::LocalDateTime => SchemaPropertyIndexKind::LocalDateTime,
        TypedIndexKind::Uuid => SchemaPropertyIndexKind::Uuid,
    }
}
