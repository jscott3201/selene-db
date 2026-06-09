//! Typed `LIST<T>` element descriptors for the closed-graph catalog.

use selene_core::{
    ByteStringType, CharacterStringType, DecimalType, PropertyValueType, Value,
    byte_string_fits_type, character_string_fits_type, decimal_fits_type,
};
use serde::{Deserialize, Serialize};

/// Persistable element-type descriptor for `LIST<T>` property declarations.
#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    Hash,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
#[rkyv(
    bytecheck(bounds(__C: rkyv::validation::ArchiveContext)),
    deserialize_bounds(__D::Error: rkyv::rancor::Source),
    serialize_bounds(__S: rkyv::ser::Writer)
)]
#[non_exhaustive]
pub enum PropertyElementType {
    /// Scalar list element type.
    Scalar(PropertyValueType),
    /// STRING list element type with a user-specified length envelope.
    CharacterString(CharacterStringType),
    /// DECIMAL list element type with a user-specified precision/scale envelope.
    Decimal(DecimalType),
    /// BYTES list element type with a user-specified length envelope.
    ByteString(ByteStringType),
    /// Nested list element type.
    List(#[rkyv(omit_bounds)] Box<PropertyElementType>),
    /// Explicitly non-null element type.
    NotNull(#[rkyv(omit_bounds)] Box<PropertyElementType>),
}

impl PropertyElementType {
    /// Return the coarse property-value type for this descriptor.
    #[must_use]
    pub const fn value_type(&self) -> PropertyValueType {
        match self {
            Self::Scalar(value_type) => *value_type,
            Self::CharacterString(_) => PropertyValueType::String,
            Self::Decimal(_) => PropertyValueType::Decimal,
            Self::ByteString(_) => PropertyValueType::Bytes,
            Self::List(_) => PropertyValueType::List,
            Self::NotNull(inner) => inner.value_type(),
        }
    }

    /// Return true when `value` belongs to this element type.
    #[must_use]
    pub fn matches(&self, value: &Value) -> bool {
        match self {
            Self::NotNull(inner) => !matches!(value, Value::Null) && inner.matches(value),
            _ if matches!(value, Value::Null) => true,
            Self::Scalar(value_type) => value_type.matches(value),
            Self::CharacterString(character_string_type) => {
                matches!(value, Value::String(value) if character_string_fits_type(value, *character_string_type))
            }
            Self::Decimal(decimal_type) => {
                matches!(value, Value::Decimal(value) if decimal_fits_type(*value, *decimal_type))
            }
            Self::ByteString(byte_string_type) => {
                matches!(value, Value::Bytes(value) if byte_string_fits_type(value, *byte_string_type))
            }
            Self::List(element_type) => match value {
                Value::List(values) => values.iter().all(|value| element_type.matches(value)),
                _ => false,
            },
        }
    }
}
