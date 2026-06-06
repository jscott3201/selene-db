//! Shared string-image normalization for numeric CAST targets.

use std::borrow::Cow;

use crate::{SourceSpan, runtime::ExecutorError};

use super::invalid_character;

pub(super) fn normalize_signed_numeric_text<'a>(
    text: &'a str,
    target: &str,
    span: SourceSpan,
) -> Result<Cow<'a, str>, ExecutorError> {
    normalize_numeric_text(text, target, SignPolicy::Signed, span)
}

pub(super) fn normalize_unsigned_numeric_text<'a>(
    text: &'a str,
    target: &str,
    span: SourceSpan,
) -> Result<Cow<'a, str>, ExecutorError> {
    normalize_numeric_text(text, target, SignPolicy::Unsigned, span)
}

enum SignPolicy {
    Signed,
    Unsigned,
}

fn normalize_numeric_text<'a>(
    text: &'a str,
    target: &str,
    sign_policy: SignPolicy,
    span: SourceSpan,
) -> Result<Cow<'a, str>, ExecutorError> {
    let trimmed = text.trim();
    if matches!(sign_policy, SignPolicy::Unsigned)
        && matches!(trimmed.as_bytes().first(), Some(b'+' | b'-'))
    {
        return Err(invalid_character(text, target, span));
    }

    if !trimmed.contains('_') {
        return Ok(Cow::Borrowed(trimmed));
    }
    if !underscores_separate_digits(trimmed) {
        return Err(invalid_character(text, target, span));
    }

    let mut normalized = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch != '_' {
            normalized.push(ch);
        }
    }
    Ok(Cow::Owned(normalized))
}

fn underscores_separate_digits(text: &str) -> bool {
    let bytes = text.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'_' {
            continue;
        }
        let has_digit_before = index > 0 && bytes[index - 1].is_ascii_digit();
        let has_digit_after = bytes.get(index + 1).is_some_and(u8::is_ascii_digit);
        if !has_digit_before || !has_digit_after {
            return false;
        }
    }
    true
}
