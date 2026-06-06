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

pub(super) fn classify_signed_numeric_text<'a>(
    text: &'a str,
    target: &str,
    span: SourceSpan,
) -> Result<NumericText<'a>, ExecutorError> {
    normalize_signed_numeric_text(text, target, span).map(classify_numeric_text)
}

pub(super) fn classify_unsigned_numeric_text<'a>(
    text: &'a str,
    target: &str,
    span: SourceSpan,
) -> Result<NumericText<'a>, ExecutorError> {
    normalize_unsigned_numeric_text(text, target, span).map(classify_numeric_text)
}

pub(super) enum NumericText<'a> {
    Integer(Cow<'a, str>),
    Decimal(Cow<'a, str>),
    Approximate(Cow<'a, str>),
}

impl<'a> NumericText<'a> {
    pub(super) fn image(&self) -> &str {
        match self {
            Self::Integer(image) | Self::Decimal(image) | Self::Approximate(image) => image,
        }
    }
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

fn classify_numeric_text<'a>(normalized: Cow<'a, str>) -> NumericText<'a> {
    let (image, suffix) = split_numeric_suffix(normalized);
    match suffix {
        Some(NumericSuffix::Approximate) => NumericText::Approximate(image),
        Some(NumericSuffix::Exact) if image.contains('.') || contains_exponent(image.as_ref()) => {
            NumericText::Decimal(image)
        }
        Some(NumericSuffix::Exact) => NumericText::Integer(image),
        None if contains_exponent(image.as_ref()) => NumericText::Approximate(image),
        None if image.contains('.') => NumericText::Decimal(image),
        None => NumericText::Integer(image),
    }
}

enum NumericSuffix {
    Exact,
    Approximate,
}

fn split_numeric_suffix<'a>(image: Cow<'a, str>) -> (Cow<'a, str>, Option<NumericSuffix>) {
    let suffix = match image.as_ref().as_bytes().last().copied() {
        Some(b'f' | b'F' | b'd' | b'D') => NumericSuffix::Approximate,
        Some(b'm' | b'M') => NumericSuffix::Exact,
        _ => return (image, None),
    };
    (strip_last_byte(image), Some(suffix))
}

fn strip_last_byte<'a>(image: Cow<'a, str>) -> Cow<'a, str> {
    match image {
        Cow::Borrowed(text) => Cow::Borrowed(&text[..text.len() - 1]),
        Cow::Owned(mut text) => {
            text.pop();
            Cow::Owned(text)
        }
    }
}

fn contains_exponent(image: &str) -> bool {
    image
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'e' | b'E'))
}
