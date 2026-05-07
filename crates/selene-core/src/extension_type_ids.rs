//! First-party extension value-type ID reservations.
//!
//! `ExtensionTypeId` is an IANA-style numeric namespace for
//! `Value::Extended { type_id, payload }` values. The first-party range is
//! `0x00000100..=0x0000FFFF`; third-party extensions use
//! `0x00010000..=0xFFFFFFFE`.

use serde::{Deserialize, Serialize};

/// Numeric ID reserving an extension value type.
///
/// Reserved ranges:
///
/// * `0x00000000..=0x000000FF` - selene-core
/// * `0x00000100..=0x0000FFFF` - first-party selene-* extensions
/// * `0x00010000..=0xFFFFFFFE` - third-party extensions
/// * `0xFFFFFFFF` - reserved sentinel
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[repr(transparent)]
pub struct ExtensionTypeId(pub u32);

impl ExtensionTypeId {
    /// First valid first-party extension ID.
    pub const FIRST_PARTY_MIN: Self = Self(0x00000100);
    /// Last valid first-party extension ID.
    pub const FIRST_PARTY_MAX: Self = Self(0x0000FFFF);
    /// First valid third-party extension ID.
    pub const THIRD_PARTY_MIN: Self = Self(0x00010000);
    /// Last valid third-party extension ID.
    pub const THIRD_PARTY_MAX: Self = Self(0xFFFFFFFE);
    /// Reserved sentinel value.
    pub const RESERVED_SENTINEL: Self = Self(0xFFFFFFFF);
}

/// Numeric ID reserved for the vector value type owned by `selene-vector`.
pub const SELENE_VECTOR: ExtensionTypeId = ExtensionTypeId(0x00000100);

/// Numeric ID reserved for a future time-series value type, if needed.
pub const SELENE_TIMESERIES: ExtensionTypeId = ExtensionTypeId(0x00000101);

/// Numeric ID reserved for a future RDF value type, if needed.
pub const SELENE_RDF: ExtensionTypeId = ExtensionTypeId(0x00000102);

/// Numeric ID reserved for a future full-text value type, if needed.
pub const SELENE_FULLTEXT: ExtensionTypeId = ExtensionTypeId(0x00000103);

/// First-party reservations by stable name.
pub const FIRST_PARTY_EXTENSION_TYPE_IDS: &[(&str, ExtensionTypeId)] = &[
    ("selene-vector.vector", SELENE_VECTOR),
    ("selene-timeseries.reserved", SELENE_TIMESERIES),
    ("selene-rdf.reserved", SELENE_RDF),
    ("selene-fulltext.reserved", SELENE_FULLTEXT),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_party_ids_in_range() {
        for &(_name, id) in FIRST_PARTY_EXTENSION_TYPE_IDS {
            assert!(id >= ExtensionTypeId::FIRST_PARTY_MIN);
            assert!(id <= ExtensionTypeId::FIRST_PARTY_MAX);
        }
    }

    #[test]
    fn extension_type_id_is_u32_sized() {
        assert_eq!(std::mem::size_of::<ExtensionTypeId>(), 4);
    }
}
