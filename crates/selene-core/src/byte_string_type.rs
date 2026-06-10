//! Byte-string length envelopes for bounded `BYTES` value types.

use serde::{Deserialize, Serialize};

/// Maximum declared byte length for bounded byte-string types
/// (`BYTES(n)` / `BYTES(min, max)` / `BINARY(n)` / `VARBINARY(n)`).
///
/// Declared lengths are durable schema metadata, and fixed-length store
/// assignment and CAST coercion *pad* values up to `min_len`, so an unbounded
/// declared length lets a read-only statement allocate arbitrarily large
/// buffers. 2^20 bytes bounds the worst-case zero-padded allocation to 1 MiB
/// while staying far above realistic schema declarations. This is the
/// implementation-defined declared-length cap in the same posture as
/// [`MAX_CHARACTER_STRING_TYPE_LENGTH`](crate::MAX_CHARACTER_STRING_TYPE_LENGTH)
/// and `MAX_DECIMAL_PRECISION`.
pub const MAX_BYTE_STRING_TYPE_LENGTH: u64 = 1 << 20;

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
    /// Construct a byte-string type when the length bounds are valid and
    /// within [`MAX_BYTE_STRING_TYPE_LENGTH`].
    #[must_use]
    pub const fn new(min_len: u64, max_len: u64) -> Option<Self> {
        if max_len == 0 || min_len > max_len || max_len > MAX_BYTE_STRING_TYPE_LENGTH {
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

#[cfg(test)]
mod tests {
    use super::{ByteStringType, MAX_BYTE_STRING_TYPE_LENGTH};

    #[test]
    fn byte_string_type_accepts_lengths_at_the_declared_cap() {
        let cap = MAX_BYTE_STRING_TYPE_LENGTH;
        assert!(ByteStringType::new(0, cap).is_some());
        assert!(ByteStringType::new(cap, cap).is_some());
        assert!(ByteStringType::new(1, 1).is_some());
    }

    #[test]
    fn byte_string_type_rejects_lengths_above_the_declared_cap() {
        let cap = MAX_BYTE_STRING_TYPE_LENGTH;
        assert!(ByteStringType::new(0, cap + 1).is_none());
        assert!(ByteStringType::new(cap + 1, cap + 1).is_none());
        assert!(ByteStringType::new(0, u64::MAX).is_none());
        assert!(ByteStringType::new(u64::MAX, u64::MAX).is_none());
    }

    #[test]
    fn byte_string_type_rejects_invalid_bounds() {
        assert!(ByteStringType::new(0, 0).is_none());
        assert!(ByteStringType::new(3, 2).is_none());
    }
}
