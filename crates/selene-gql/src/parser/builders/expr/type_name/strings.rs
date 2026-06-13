//! Character and byte string type-name builders.

use crate::{
    ast::{
        ByteStringType, ByteStringTypeForm, CharacterStringType, CharacterStringTypeForm, GqlType,
        MAX_BYTE_STRING_TYPE_LENGTH, MAX_CHARACTER_STRING_TYPE_LENGTH, SourceSpan,
    },
    error::ParserError,
    parser::builders::{keyword_starts_with, keyword_tokens_eq},
};

pub(super) fn build_character_string_type_name(
    text: &str,
    span: SourceSpan,
) -> Result<GqlType, ParserError> {
    if !text.contains('(') {
        if keyword_tokens_eq(text, &["CHAR"]) {
            return character_string_type(1, 1, CharacterStringTypeForm::CharFixed, span);
        }
        return Ok(GqlType::String);
    }
    let lengths = parse_string_type_lengths(text, span, "character string")?;
    if keyword_starts_with(text, "CHAR") {
        let [length] = string_type_single_length(&lengths, span, "character string")?;
        return character_string_type(length, length, CharacterStringTypeForm::CharFixed, span);
    }
    if keyword_starts_with(text, "VARCHAR") {
        let [max] = string_type_single_length(&lengths, span, "character string")?;
        return character_string_type(0, max, CharacterStringTypeForm::VarcharMax, span);
    }
    match lengths.as_slice() {
        [max] => character_string_type(0, *max, CharacterStringTypeForm::StringMax, span),
        [min, max] => {
            character_string_type(*min, *max, CharacterStringTypeForm::StringMinMax, span)
        }
        _ => Err(ParserError::syntax(
            "character string type expects one or two length bounds",
            span,
            None,
        )),
    }
}

pub(super) fn build_byte_string_type_name(
    text: &str,
    span: SourceSpan,
) -> Result<GqlType, ParserError> {
    if !text.contains('(') {
        return Ok(GqlType::Bytes);
    }
    let lengths = parse_string_type_lengths(text, span, "byte string")?;
    if keyword_starts_with(text, "BINARY") {
        let [length] = byte_string_single_length(&lengths, span)?;
        return byte_string_type(length, length, ByteStringTypeForm::BinaryFixed, span);
    }
    if keyword_starts_with(text, "VARBINARY") {
        let [max] = byte_string_single_length(&lengths, span)?;
        return byte_string_type(0, max, ByteStringTypeForm::VarbinaryMax, span);
    }
    match lengths.as_slice() {
        [max] => byte_string_type(0, *max, ByteStringTypeForm::BytesMax, span),
        [min, max] => byte_string_type(*min, *max, ByteStringTypeForm::BytesMinMax, span),
        _ => Err(ParserError::syntax(
            "byte string type expects one or two length bounds",
            span,
            None,
        )),
    }
}

fn parse_string_type_lengths(
    text: &str,
    span: SourceSpan,
    kind: &'static str,
) -> Result<Vec<u64>, ParserError> {
    let open = text.find('(').ok_or_else(|| {
        ParserError::syntax(format!("{kind} type is missing length bounds"), span, None)
    })?;
    let close = text.rfind(')').ok_or_else(|| {
        ParserError::syntax(
            format!("{kind} type is missing closing parenthesis"),
            span,
            None,
        )
    })?;
    text[open + 1..close]
        .split(',')
        .map(str::trim)
        .map(|part| parse_unsigned_length(part, span, kind))
        .collect()
}

pub(super) fn parse_unsigned_length(
    text: &str,
    span: SourceSpan,
    kind: &'static str,
) -> Result<u64, ParserError> {
    let (digits, radix) = if let Some(rest) = text.strip_prefix("0x") {
        (rest, 16)
    } else if let Some(rest) = text.strip_prefix("0o") {
        (rest, 8)
    } else if let Some(rest) = text.strip_prefix("0b") {
        (rest, 2)
    } else {
        (text, 10)
    };
    validate_unsigned_integer_underscores(digits, span, kind)?;
    let normalized = digits.replace('_', "");
    u64::from_str_radix(&normalized, radix).map_err(|_| {
        ParserError::syntax(format!("{kind} length exceeds supported range"), span, None)
    })
}

fn validate_unsigned_integer_underscores(
    text: &str,
    span: SourceSpan,
    kind: &'static str,
) -> Result<(), ParserError> {
    let mut prev_underscore = false;
    for &byte in text.as_bytes() {
        if byte == b'_' {
            if prev_underscore {
                return Err(ParserError::syntax(
                    format!("{kind} length contains consecutive underscores"),
                    span,
                    Some("use `_` only between digits".into()),
                ));
            }
            prev_underscore = true;
        } else {
            prev_underscore = false;
        }
    }
    if prev_underscore {
        return Err(ParserError::syntax(
            format!("{kind} length cannot end with an underscore"),
            span,
            Some("remove the trailing `_`".into()),
        ));
    }
    Ok(())
}

fn byte_string_single_length(lengths: &[u64], span: SourceSpan) -> Result<[u64; 1], ParserError> {
    string_type_single_length(lengths, span, "byte string")
}

fn string_type_single_length(
    lengths: &[u64],
    span: SourceSpan,
    kind: &'static str,
) -> Result<[u64; 1], ParserError> {
    match lengths {
        [length] => Ok([*length]),
        _ => Err(ParserError::syntax(
            format!("{kind} type expects exactly one length bound"),
            span,
            None,
        )),
    }
}

fn character_string_type(
    min_len: u64,
    max_len: u64,
    form: CharacterStringTypeForm,
    span: SourceSpan,
) -> Result<GqlType, ParserError> {
    // Fixed-length coercion pads values up to `min_len`, so an unbounded
    // declared length is an allocation primitive in read-only statements.
    // Checking `max_len` alone is sufficient: `new` rejects min > max.
    if max_len > MAX_CHARACTER_STRING_TYPE_LENGTH {
        return Err(ParserError::syntax(
            "character string length exceeds the implementation-defined maximum",
            span,
            Some(format!(
                "selene-db currently supports declared character string lengths up to {MAX_CHARACTER_STRING_TYPE_LENGTH} characters"
            )),
        ));
    }
    CharacterStringType::new(min_len, max_len, form)
        .map(GqlType::CharacterString)
        .ok_or_else(|| {
            ParserError::syntax(
                "character string length bounds require max > 0 and min <= max",
                span,
                None,
            )
        })
}

fn byte_string_type(
    min_len: u64,
    max_len: u64,
    form: ByteStringTypeForm,
    span: SourceSpan,
) -> Result<GqlType, ParserError> {
    // Fixed-length coercion zero-pads values up to `min_len`, so an unbounded
    // declared length is an allocation primitive in read-only statements.
    // Checking `max_len` alone is sufficient: `new` rejects min > max.
    if max_len > MAX_BYTE_STRING_TYPE_LENGTH {
        return Err(ParserError::syntax(
            "byte string length exceeds the implementation-defined maximum",
            span,
            Some(format!(
                "selene-db currently supports declared byte string lengths up to {MAX_BYTE_STRING_TYPE_LENGTH} bytes"
            )),
        ));
    }
    ByteStringType::new(min_len, max_len, form)
        .map(GqlType::ByteString)
        .ok_or_else(|| {
            ParserError::syntax(
                "byte string length bounds require max > 0 and min <= max",
                span,
                None,
            )
        })
}
