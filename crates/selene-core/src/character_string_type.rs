//! Character-string length envelopes for bounded `STRING` value types.

use serde::{Deserialize, Serialize};

use crate::DbString;

/// Maximum declared character length for bounded character-string types
/// (`STRING(n)` / `STRING(min, max)` / `CHAR(n)` / `VARCHAR(n)`).
///
/// Declared lengths are durable schema metadata, and fixed-length store
/// assignment and CAST coercion *pad* values up to `min_len`, so an unbounded
/// declared length lets a read-only statement allocate arbitrarily large
/// buffers. 2^20 characters bounds the worst-case padded allocation to a few
/// MiB (the store-assignment funnel pads through a 4-byte-per-char
/// `Vec<char>` intermediate) while staying far above realistic schema
/// declarations. This is the implementation-defined declared-length cap in
/// the same posture as `MAX_DECIMAL_PRECISION` and the `IL013` per-string
/// byte cap ([`crate::db_string::MAX_DB_STRING_BYTES`]), which guards stored
/// string bytes but not declared-length padding.
pub const MAX_CHARACTER_STRING_TYPE_LENGTH: u64 = 1 << 20;

/// User-specified character-string length metadata.
#[derive(
    Clone,
    Copy,
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
pub struct CharacterStringType {
    /// Minimum character length accepted by the type.
    pub min_len: u64,
    /// Maximum character length accepted by the type.
    pub max_len: u64,
}

impl CharacterStringType {
    /// Construct a character-string type when the length bounds are valid
    /// and within [`MAX_CHARACTER_STRING_TYPE_LENGTH`].
    #[must_use]
    pub const fn new(min_len: u64, max_len: u64) -> Option<Self> {
        if max_len == 0 || min_len > max_len || max_len > MAX_CHARACTER_STRING_TYPE_LENGTH {
            return None;
        }
        Some(Self { min_len, max_len })
    }

    /// Return true if this type accepts only one character length.
    #[must_use]
    pub const fn is_fixed_length(&self) -> bool {
        self.min_len == self.max_len
    }

    /// Return true when `len` belongs to this character-string envelope.
    #[must_use]
    pub fn matches_len(self, len: usize) -> bool {
        match u64::try_from(len) {
            Ok(len) => self.min_len <= len && len <= self.max_len,
            Err(_) => false,
        }
    }
}

/// Return true when `value` can be represented by `character_string_type`.
#[must_use]
pub fn character_string_fits_type(
    value: &DbString,
    character_string_type: CharacterStringType,
) -> bool {
    character_string_type.matches_len(value.as_str().chars().count())
}

#[cfg(test)]
mod tests {
    use super::{CharacterStringType, MAX_CHARACTER_STRING_TYPE_LENGTH};

    #[test]
    fn character_string_type_accepts_lengths_at_the_declared_cap() {
        let cap = MAX_CHARACTER_STRING_TYPE_LENGTH;
        assert!(CharacterStringType::new(0, cap).is_some());
        assert!(CharacterStringType::new(cap, cap).is_some());
        assert!(CharacterStringType::new(1, 1).is_some());
    }

    #[test]
    fn character_string_type_rejects_lengths_above_the_declared_cap() {
        let cap = MAX_CHARACTER_STRING_TYPE_LENGTH;
        assert!(CharacterStringType::new(0, cap + 1).is_none());
        assert!(CharacterStringType::new(cap + 1, cap + 1).is_none());
        assert!(CharacterStringType::new(0, u64::MAX).is_none());
        assert!(CharacterStringType::new(u64::MAX, u64::MAX).is_none());
    }

    #[test]
    fn character_string_type_rejects_invalid_bounds() {
        assert!(CharacterStringType::new(0, 0).is_none());
        assert!(CharacterStringType::new(3, 2).is_none());
    }
}
