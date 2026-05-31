//! Value-type ID reservations for `Value::Extended` payloads.
//!
//! `ExtensionTypeId` is an IANA-style numeric namespace for
//! `Value::Extended { type_id, payload }` values. selene-db is a single native
//! engine with no loadable extensions; the upper ID range is reserved for
//! value types defined by externalized sister projects (time-series, RDF,
//! vectors, GraphRAG) that carry their own opaque payloads through this engine.
//! The first-party range is `0x00000100..=0x0000FFFF`; sister-project value
//! types use `0x00010000..=0xFFFFFFFE`.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Numeric ID reserving an [`crate::Value::Extended`] value type.
///
/// Reserved ranges:
///
/// * `0x00000000..=0x000000FF` - selene-core
/// * `0x00000100..=0x0000FFFF` - first-party selene-* value types
/// * `0x00010000..=0xFFFFFFFE` - externalized sister-project value types
/// * `0xFFFFFFFF` - reserved sentinel
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[repr(transparent)]
pub struct ExtensionTypeId(pub u32);

impl ExtensionTypeId {
    /// First valid first-party value-type ID.
    pub const FIRST_PARTY_MIN: Self = Self(0x00000100);
    /// Last valid first-party value-type ID.
    pub const FIRST_PARTY_MAX: Self = Self(0x0000FFFF);
    /// First valid sister-project value-type ID.
    pub const SISTER_PROJECT_MIN: Self = Self(0x00010000);
    /// Last valid sister-project value-type ID.
    pub const SISTER_PROJECT_MAX: Self = Self(0xFFFFFFFE);
    /// Reserved sentinel value.
    pub const RESERVED_SENTINEL: Self = Self(0xFFFFFFFF);

    /// Return the registered symbolic name for this extension type ID.
    #[must_use]
    pub fn symbolic_name(&self) -> Option<&'static str> {
        FIRST_PARTY_EXTENSION_TYPE_IDS
            .iter()
            .find_map(|(name, id)| (id == self).then_some(*name))
    }
}

impl fmt::Display for ExtensionTypeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(name) = self.symbolic_name() {
            formatter.write_str(name)
        } else {
            write!(formatter, "{:#010x}", self.0)
        }
    }
}

/// Numeric ID reserved for a future time-series value type, if needed.
pub const SELENE_TIMESERIES: ExtensionTypeId = ExtensionTypeId(0x00000101);

/// Numeric ID reserved for a future RDF value type, if needed.
pub const SELENE_RDF: ExtensionTypeId = ExtensionTypeId(0x00000102);

/// First-party reservations by stable name.
pub const FIRST_PARTY_EXTENSION_TYPE_IDS: &[(&str, ExtensionTypeId)] = &[
    ("selene-timeseries.reserved", SELENE_TIMESERIES),
    ("selene-rdf.reserved", SELENE_RDF),
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

    #[test]
    fn display_unknown_id_uses_hex() {
        assert_eq!(ExtensionTypeId(0xDEAD_BEEF).to_string(), "0xdeadbeef");
    }

    #[test]
    fn display_sister_project_unknown_id_uses_hex() {
        assert_eq!(ExtensionTypeId(0x0001_FFFF).to_string(), "0x0001ffff");
        assert_eq!(
            ExtensionTypeId::SISTER_PROJECT_MIN.to_string(),
            "0x00010000"
        );
    }

    #[test]
    fn symbolic_name_round_trips_against_first_party_table() {
        for &(name, id) in FIRST_PARTY_EXTENSION_TYPE_IDS {
            assert_eq!(id.symbolic_name(), Some(name));
        }
    }
}
