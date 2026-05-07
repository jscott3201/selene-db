//! First-party extension value-type ID reservations.
//!
//! `ExtensionTypeId` is an IANA-style numeric namespace for
//! `Value::Extended { type_id, payload }` values. The first-party range is
//! `0x00000100..=0x0000FFFF`; third-party extensions use
//! `0x00010000..=0xFFFFFFFE`.

/// Numeric ID reserved for the vector value type owned by `selene-vector`.
pub const SELENE_VECTOR: u32 = 0x00000100;

/// Numeric ID reserved for a future time-series value type, if needed.
pub const SELENE_TIMESERIES: u32 = 0x00000101;

/// Numeric ID reserved for a future RDF value type, if needed.
pub const SELENE_RDF: u32 = 0x00000102;

/// Numeric ID reserved for a future full-text value type, if needed.
pub const SELENE_FULLTEXT: u32 = 0x00000103;

/// First-party reservations by stable name.
pub const FIRST_PARTY_EXTENSION_TYPE_IDS: &[(&str, u32)] = &[
    ("selene-vector.vector", SELENE_VECTOR),
    ("selene-timeseries.reserved", SELENE_TIMESERIES),
    ("selene-rdf.reserved", SELENE_RDF),
    ("selene-fulltext.reserved", SELENE_FULLTEXT),
];
