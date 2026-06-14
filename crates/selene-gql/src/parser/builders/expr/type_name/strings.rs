//! Character and byte string type-name builders.

use pest::iterators::Pair;

use crate::{
    ast::{
        ByteStringType, ByteStringTypeForm, CharacterStringType, CharacterStringTypeForm, GqlType,
        MAX_BYTE_STRING_TYPE_LENGTH, MAX_CHARACTER_STRING_TYPE_LENGTH, SourceSpan,
    },
    error::ParserError,
    parser::builders::{keyword_starts_with, span, unexpected_pair},
};

use super::Rule;

pub(super) fn build_character_string_type_name(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<GqlType, ParserError> {
    let text = pair.as_str();
    let lengths = collect_string_type_lengths(
        pair,
        Rule::character_string_length,
        source_span,
        "character string",
    )?;
    if lengths.is_empty() {
        if keyword_starts_with(text, "CHAR") {
            return character_string_type(1, 1, CharacterStringTypeForm::CharFixed, source_span);
        }
        return Ok(GqlType::String);
    }
    if keyword_starts_with(text, "CHAR") {
        let [length] = string_type_single_length(&lengths, source_span, "character string")?;
        return character_string_type(
            length,
            length,
            CharacterStringTypeForm::CharFixed,
            source_span,
        );
    }
    if keyword_starts_with(text, "VARCHAR") {
        let [max] = string_type_single_length(&lengths, source_span, "character string")?;
        return character_string_type(0, max, CharacterStringTypeForm::VarcharMax, source_span);
    }
    match lengths.as_slice() {
        [max] => character_string_type(0, *max, CharacterStringTypeForm::StringMax, source_span),
        [min, max] => character_string_type(
            *min,
            *max,
            CharacterStringTypeForm::StringMinMax,
            source_span,
        ),
        _ => Err(ParserError::syntax(
            "character string type expects one or two length bounds",
            source_span,
            None,
        )),
    }
}

pub(super) fn build_byte_string_type_name(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<GqlType, ParserError> {
    let text = pair.as_str();
    let lengths =
        collect_string_type_lengths(pair, Rule::byte_string_length, source_span, "byte string")?;
    if lengths.is_empty() {
        return Ok(GqlType::Bytes);
    }
    if keyword_starts_with(text, "BINARY") {
        let [length] = byte_string_single_length(&lengths, source_span)?;
        return byte_string_type(length, length, ByteStringTypeForm::BinaryFixed, source_span);
    }
    if keyword_starts_with(text, "VARBINARY") {
        let [max] = byte_string_single_length(&lengths, source_span)?;
        return byte_string_type(0, max, ByteStringTypeForm::VarbinaryMax, source_span);
    }
    match lengths.as_slice() {
        [max] => byte_string_type(0, *max, ByteStringTypeForm::BytesMax, source_span),
        [min, max] => byte_string_type(*min, *max, ByteStringTypeForm::BytesMinMax, source_span),
        _ => Err(ParserError::syntax(
            "byte string type expects one or two length bounds",
            source_span,
            None,
        )),
    }
}

fn collect_string_type_lengths(
    pair: Pair<'_, Rule>,
    length_rule: Rule,
    source_span: SourceSpan,
    kind: &'static str,
) -> Result<Vec<u64>, ParserError> {
    let mut lengths = Vec::new();
    for child in pair.into_inner() {
        if child.as_rule() == length_rule {
            lengths.push(parse_unsigned_length(child.as_str(), span(&child), kind)?);
        } else if !matches!(
            child.as_rule(),
            Rule::character_string_type_string_kw
                | Rule::character_string_type_char_kw
                | Rule::character_string_type_varchar_kw
                | Rule::byte_string_type_bytes_kw
                | Rule::byte_string_type_binary_kw
                | Rule::byte_string_type_varbinary_kw
        ) {
            return Err(unexpected_pair(child, "unexpected string type child"));
        }
    }
    if lengths.len() > 2 {
        return Err(ParserError::syntax(
            format!("{kind} type expects one or two length bounds"),
            source_span,
            None,
        ));
    }
    Ok(lengths)
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
