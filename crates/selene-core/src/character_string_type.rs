//! Character-string length envelopes for bounded `STRING` value types.

use serde::{Deserialize, Serialize};

use crate::DbString;

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
    /// Construct a character-string type when the length bounds are valid.
    #[must_use]
    pub const fn new(min_len: u64, max_len: u64) -> Option<Self> {
        if max_len == 0 || min_len > max_len {
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
