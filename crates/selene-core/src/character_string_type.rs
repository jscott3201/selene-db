//! Character-string length envelopes for bounded `STRING` value types.

use std::borrow::Cow;

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

/// Failure modes from [`coerce_character_string_to_type`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CharacterStringCoercionError {
    /// The source character length exceeds the supported length range.
    SourceLengthOverflow,
    /// The target minimum length exceeds the supported length range.
    TargetMinOverflow,
    /// The target maximum length exceeds the supported length range.
    TargetMaxOverflow,
    /// Truncation would discard a character outside the
    /// [`is_truncating_whitespace`] subset.
    NonSpaceTruncation,
}

/// Return true when `character` may be silently discarded by length-bound
/// truncation.
///
/// This is the engine-wide implementation-defined IV023 `<truncating
/// whitespace>` subset (ISO/IEC 39075:2024 §21.3 SR 7, §22.10): U+0020 SPACE
/// only. Tabs, newlines, NBSP, and every other Unicode whitespace character
/// are data-bearing and raise 22001 `string data, right truncation` instead
/// of truncating. Character CAST, store assignment, DEFAULT descriptor
/// coercion, and length-capped concatenation all share this predicate; do not
/// fork the policy per funnel.
#[must_use]
pub const fn is_truncating_whitespace(character: char) -> bool {
    character == ' '
}

/// Coerce `value` into the `target` character-length envelope.
///
/// Sources shorter than the minimum length are right-padded with U+0020.
/// Sources longer than the maximum length truncate only when every discarded
/// character satisfies [`is_truncating_whitespace`]; any other discarded
/// character is [`CharacterStringCoercionError::NonSpaceTruncation`] so every
/// funnel raises 22001 `string data, right truncation` for the same inputs.
pub fn coerce_character_string_to_type(
    value: &str,
    target: CharacterStringType,
) -> Result<Cow<'_, str>, CharacterStringCoercionError> {
    let len = value.chars().count();
    let len_u64 =
        u64::try_from(len).map_err(|_| CharacterStringCoercionError::SourceLengthOverflow)?;
    if len_u64 >= target.min_len && len_u64 <= target.max_len {
        return Ok(Cow::Borrowed(value));
    }
    if len_u64 < target.min_len {
        let min_len = usize::try_from(target.min_len)
            .map_err(|_| CharacterStringCoercionError::TargetMinOverflow)?;
        let mut padded = String::with_capacity(value.len().saturating_add(min_len - len));
        padded.push_str(value);
        padded.extend(std::iter::repeat_n(' ', min_len - len));
        return Ok(Cow::Owned(padded));
    }
    let max_len = usize::try_from(target.max_len)
        .map_err(|_| CharacterStringCoercionError::TargetMaxOverflow)?;
    let truncate_at = value
        .char_indices()
        .nth(max_len)
        .map(|(offset, _)| offset)
        .unwrap_or(value.len());
    if value[truncate_at..]
        .chars()
        .any(|character| !is_truncating_whitespace(character))
    {
        return Err(CharacterStringCoercionError::NonSpaceTruncation);
    }
    Ok(Cow::Borrowed(&value[..truncate_at]))
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::{
        CharacterStringCoercionError, CharacterStringType, MAX_CHARACTER_STRING_TYPE_LENGTH,
        coerce_character_string_to_type, is_truncating_whitespace,
    };

    fn envelope(min_len: u64, max_len: u64) -> CharacterStringType {
        CharacterStringType::new(min_len, max_len).expect("test envelope is valid")
    }

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

    #[test]
    fn truncating_whitespace_is_space_only() {
        assert!(is_truncating_whitespace(' '));
        // The IV023 subset excludes every other Rust `char::is_whitespace`
        // member: those characters are data-bearing under truncation.
        for character in [
            '\t', '\n', '\r', '\u{000B}', '\u{000C}', '\u{0085}', '\u{00A0}', '\u{2002}',
            '\u{2028}', '\u{3000}',
        ] {
            assert!(character.is_whitespace(), "{character:?} probes whitespace");
            assert!(
                !is_truncating_whitespace(character),
                "{character:?} must be data-bearing"
            );
        }
    }

    #[test]
    fn in_envelope_borrows_unchanged() {
        assert_eq!(
            coerce_character_string_to_type("ab", envelope(1, 4)),
            Ok(Cow::Borrowed("ab"))
        );
        // Trailing non-space whitespace inside the envelope is preserved, not
        // policed: the policy only constrains discarded characters.
        assert_eq!(
            coerce_character_string_to_type("a\t", envelope(1, 4)),
            Ok(Cow::Borrowed("a\t"))
        );
    }

    #[test]
    fn below_minimum_pads_with_space() {
        assert_eq!(
            coerce_character_string_to_type("a", envelope(3, 3)).as_deref(),
            Ok("a  ")
        );
        assert_eq!(
            coerce_character_string_to_type("", envelope(2, 4)).as_deref(),
            Ok("  ")
        );
    }

    #[test]
    fn truncation_discards_spaces_only() {
        assert_eq!(
            coerce_character_string_to_type("ab  ", envelope(1, 2)),
            Ok(Cow::Borrowed("ab"))
        );
        for suffix in ["\t", "\n", "\u{00A0}", " \t ", "x "] {
            let source = format!("ab{suffix}");
            assert_eq!(
                coerce_character_string_to_type(&source, envelope(1, 2)),
                Err(CharacterStringCoercionError::NonSpaceTruncation),
                "suffix {suffix:?} must reject"
            );
        }
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        // Multi-byte characters inside the kept prefix must not skew the
        // truncation offset.
        assert_eq!(
            coerce_character_string_to_type("é½  ", envelope(1, 2)),
            Ok(Cow::Borrowed("é½"))
        );
        assert_eq!(
            coerce_character_string_to_type("é½ \u{00A0}", envelope(1, 2)),
            Err(CharacterStringCoercionError::NonSpaceTruncation)
        );
    }
}
