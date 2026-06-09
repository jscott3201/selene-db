//! Byte-string length envelopes for bounded `BYTES` value types.

use serde::{Deserialize, Serialize};

/// User-specified byte-string length metadata.
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
pub struct ByteStringType {
    /// Minimum byte length accepted by the type.
    pub min_len: u64,
    /// Maximum byte length accepted by the type.
    pub max_len: u64,
}

impl ByteStringType {
    /// Construct a byte-string type when the length bounds are valid.
    #[must_use]
    pub const fn new(min_len: u64, max_len: u64) -> Option<Self> {
        if max_len == 0 || min_len > max_len {
            return None;
        }
        Some(Self { min_len, max_len })
    }

    /// Return true if this type accepts only one byte length.
    #[must_use]
    pub const fn is_fixed_length(&self) -> bool {
        self.min_len == self.max_len
    }

    /// Return true when `len` belongs to this byte-string envelope.
    #[must_use]
    pub fn matches_len(self, len: usize) -> bool {
        match u64::try_from(len) {
            Ok(len) => self.min_len <= len && len <= self.max_len,
            Err(_) => false,
        }
    }
}

/// Return true when `value` can be represented by `byte_string_type`.
#[must_use]
pub fn byte_string_fits_type(value: &[u8], byte_string_type: ByteStringType) -> bool {
    byte_string_type.matches_len(value.len())
}
