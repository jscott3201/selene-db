//! Literal expression builders.

use std::sync::Arc;

use pest::iterators::Pair;
use rust_decimal::Decimal;
use selene_core::DbString;

use crate::{
    GqlStatus,
    ast::{
        CharacterStringLiteralKind, DecimalLiteralKind, FloatLiteralKind, IntegerLiteralKind,
        Literal, SourceSpan, ValueExpr,
    },
    error::ParserError,
    temporal_parse::{self, ParsedDateTime, ParsedTime},
};

use super::{Rule, build_value_expr};
use crate::parser::builders::{db_string_from_owned, first_child, not_implemented, span};

pub(super) fn build_literal_expr(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::literal);
    let child = first_child(pair)?;
    if child.as_rule() == Rule::list_lit {
        return build_list_lit(child);
    }
    build_literal_child(child).map(ValueExpr::Literal)
}

pub(super) fn build_list_lit(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    Ok(ValueExpr::ListLiteral {
        items: build_list_items(pair)?,
        span: source_span,
    })
}

pub(super) fn build_list_items(pair: Pair<'_, Rule>) -> Result<Vec<ValueExpr>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::expr)
        .map(|child| build_value_expr(child))
        .collect()
}

pub(super) fn parse_string_pair_with_kind(
    pair: Pair<'_, Rule>,
) -> Result<(DbString, CharacterStringLiteralKind), ParserError> {
    let string_span = span(&pair);
    let parsed = parse_character_string(pair.as_str(), string_span)?;
    let db_string_value = db_string_from_owned(parsed.value, string_span, "string literal")?;
    Ok((db_string_value, parsed.kind))
}

/// Decode a `string_lit` pair into its raw (unquoted, unescaped) text.
///
/// Used by surfaces that need the decoded string value rather than a `DbString`
/// literal — for example the `SESSION SET TIME ZONE '<region>'` time-zone
/// string (ISO/IEC 39075:2024 section 7.1).
pub(super) fn decode_string_text_with_kind(
    pair: &Pair<'_, Rule>,
) -> Result<(String, CharacterStringLiteralKind), ParserError> {
    let parsed = parse_character_string(pair.as_str(), span(pair))?;
    Ok((parsed.value, parsed.kind))
}

pub(super) fn with_numeric_span(value: ValueExpr, source_span: SourceSpan) -> ValueExpr {
    match value {
        ValueExpr::Literal(Literal::Integer(value, _)) => {
            ValueExpr::Literal(Literal::Integer(value, source_span))
        }
        ValueExpr::Literal(Literal::RadixInteger(value, _, kind)) => {
            ValueExpr::Literal(Literal::RadixInteger(value, source_span, kind))
        }
        ValueExpr::Literal(Literal::Decimal(value, _, kind)) => {
            ValueExpr::Literal(Literal::Decimal(value, source_span, kind))
        }
        ValueExpr::Literal(Literal::Float(value, _, kind)) => {
            ValueExpr::Literal(Literal::Float(value, source_span, kind))
        }
        other => other,
    }
}

fn build_literal_child(child: Pair<'_, Rule>) -> Result<Literal, ParserError> {
    let child_span = span(&child);
    match child.as_rule() {
        Rule::null_lit => Ok(Literal::Null(child_span)),
        // Per ISO/IEC 39075:2024 §21.2 <boolean literal> ::= TRUE | FALSE |
        // UNKNOWN. UNKNOWN is the boolean unknown truth value; the runtime
        // represents it as `Value::Null` (validated three-valued logic), so the
        // parser lowers it to the same `Literal::Null` node as `NULL`.
        Rule::unknown_lit => Ok(Literal::Null(child_span)),
        Rule::bool_lit => Ok(Literal::Bool(
            child.as_str().eq_ignore_ascii_case("true"),
            child_span,
        )),
        Rule::int_lit => parse_i64(child.as_str(), child_span),
        Rule::decimal_lit => parse_decimal(child.as_str(), child_span),
        Rule::float_lit => parse_f64(child.as_str(), child_span),
        Rule::byte_string_lit => parse_byte_string_lit(child.as_str(), child_span),
        Rule::string_lit => parse_string(child.as_str(), child_span),
        Rule::uuid_lit => parse_uuid_lit(child, child_span),
        Rule::date_lit => parse_date_lit(child, child_span),
        Rule::local_datetime_lit => parse_local_datetime_lit(child, child_span),
        Rule::zoned_datetime_lit => parse_zoned_datetime_lit(child, child_span),
        Rule::datetime_bare_lit => parse_datetime_lit(child, child_span),
        Rule::local_time_lit => parse_local_time_lit(child, child_span),
        Rule::zoned_time_lit => parse_zoned_time_lit(child, child_span),
        Rule::time_lit => parse_time_lit(child, child_span),
        Rule::duration_lit => parse_duration_lit(child, child_span),
        _ => Err(not_implemented(
            &child,
            "literal builder lands in a later brief",
        )),
    }
}

fn parse_uuid_lit(pair: Pair<'_, Rule>, source_span: SourceSpan) -> Result<Literal, ParserError> {
    let string_pair = first_child(pair)?;
    let parsed = parse_character_string(string_pair.as_str(), span(&string_pair))?;
    uuid::Uuid::parse_str(&parsed.value)
        .map(|uuid| Literal::Uuid(uuid, source_span, parsed.kind))
        .map_err(|error| {
            ParserError::syntax(format!("invalid UUID literal: {error}"), source_span, None)
        })
}

fn parse_date_lit(pair: Pair<'_, Rule>, source_span: SourceSpan) -> Result<Literal, ParserError> {
    let parsed = temporal_text(pair)?;
    temporal_parse::parse_date(&parsed.value)
        .map(|date| Literal::Date(date, source_span, parsed.kind))
        .map_err(|error| temporal_message(error, source_span))
}

fn parse_local_datetime_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let parsed = temporal_text(pair)?;
    temporal_parse::parse_local_datetime(&parsed.value)
        .map(|datetime| Literal::LocalDateTime(datetime, source_span, parsed.kind))
        .map_err(|error| temporal_message(error, source_span))
}

fn parse_zoned_datetime_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let parsed = temporal_text(pair)?;
    temporal_parse::parse_zoned_datetime(&parsed.value)
        .map(|zoned| Literal::ZonedDateTime(Box::new(zoned), source_span, parsed.kind))
        .map_err(|error| temporal_message(error, source_span))
}

fn parse_datetime_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let parsed = temporal_text(pair)?;
    temporal_parse::parse_datetime(&parsed.value)
        .map(|datetime| match datetime {
            ParsedDateTime::Zoned(zoned) => {
                Literal::ZonedDateTime(Box::new(zoned), source_span, parsed.kind)
            }
            ParsedDateTime::Local(datetime) => {
                Literal::LocalDateTime(datetime, source_span, parsed.kind)
            }
        })
        .map_err(|error| temporal_message(error, source_span))
}

fn parse_local_time_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let parsed = temporal_text(pair)?;
    temporal_parse::parse_local_time(&parsed.value)
        .map(|time| Literal::LocalTime(time, source_span, parsed.kind))
        .map_err(|error| temporal_message(error, source_span))
}

fn parse_zoned_time_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let parsed = temporal_text(pair)?;
    temporal_parse::parse_zoned_time(&parsed.value)
        .map(|zoned| Literal::ZonedTime(Box::new(zoned), source_span, parsed.kind))
        .map_err(|error| temporal_message(error, source_span))
}

fn parse_time_lit(pair: Pair<'_, Rule>, source_span: SourceSpan) -> Result<Literal, ParserError> {
    let parsed = temporal_text(pair)?;
    temporal_parse::parse_time(&parsed.value)
        .map(|time| match time {
            ParsedTime::Zoned(zoned) => {
                Literal::ZonedTime(Box::new(zoned), source_span, parsed.kind)
            }
            ParsedTime::Local(time) => Literal::LocalTime(time, source_span, parsed.kind),
        })
        .map_err(|error| temporal_message(error, source_span))
}

fn parse_duration_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let parsed = temporal_text(pair)?;
    temporal_parse::parse_duration(&parsed.value)
        .map(|span| Literal::Duration(Box::new(span), source_span, parsed.kind))
        .map_err(|error| duration_message(error, source_span))
}

fn parse_i64(text: &str, span: SourceSpan) -> Result<Literal, ParserError> {
    let (sign, unsigned) = split_sign(text);
    let (digits, radix, kind) = split_radix(unsigned);
    validate_underscores(digits, span)?;
    let normalized = digits.replace('_', "");
    let magnitude = i64::from_str_radix(&normalized, radix).map_err(|error| {
        ParserError::syntax(
            format!("invalid integer literal: {error}"),
            span,
            Some("integer literals must fit in i64".into()),
        )
    })?;
    let value = if sign == Sign::Negative {
        magnitude.checked_neg().ok_or_else(|| {
            ParserError::syntax(
                "integer literal overflows i64 after negation",
                span,
                Some("integer literals must fit in i64".into()),
            )
        })?
    } else {
        magnitude
    };
    Ok(match kind {
        Some(kind) => Literal::RadixInteger(value, span, kind),
        None => Literal::Integer(value, span),
    })
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Sign {
    Positive,
    Negative,
}

fn split_sign(text: &str) -> (Sign, &str) {
    if let Some(rest) = text.strip_prefix('-') {
        (Sign::Negative, rest)
    } else if let Some(rest) = text.strip_prefix('+') {
        (Sign::Positive, rest)
    } else {
        (Sign::Positive, text)
    }
}

fn split_radix(text: &str) -> (&str, u32, Option<IntegerLiteralKind>) {
    if let Some(rest) = text.strip_prefix("0x") {
        (rest, 16, Some(IntegerLiteralKind::Hexadecimal))
    } else if let Some(rest) = text.strip_prefix("0o") {
        (rest, 8, Some(IntegerLiteralKind::Octal))
    } else if let Some(rest) = text.strip_prefix("0b") {
        (rest, 2, Some(IntegerLiteralKind::Binary))
    } else {
        (text, 10, None)
    }
}

fn parse_f64(text: &str, span: SourceSpan) -> Result<Literal, ParserError> {
    let kind = classify_float_literal(text);
    let trimmed = text.strip_suffix(['f', 'd', 'F', 'D']).unwrap_or(text);
    validate_underscores(trimmed, span)?;
    let normalized = normalize_float_image(&trimmed.replace('_', ""));
    normalized
        .parse::<f64>()
        .map(|value| Literal::Float(value, span, kind))
        .map_err(|error| {
            ParserError::syntax(
                format!("invalid floating-point literal: {error}"),
                span,
                None,
            )
        })
}

fn parse_decimal(text: &str, span: SourceSpan) -> Result<Literal, ParserError> {
    let kind = classify_decimal_literal(text);
    let trimmed = text.strip_suffix(['m', 'M']).unwrap_or(text);
    validate_underscores(trimmed, span)?;
    let normalized = normalize_decimal_image(&trimmed.replace('_', ""));
    let parsed = if contains_exponent(&normalized) {
        Decimal::from_scientific(&normalized)
    } else {
        normalized.parse::<Decimal>()
    };
    parsed
        .map(|value| Literal::Decimal(value, span, kind))
        .map_err(|error| {
            ParserError::syntax(
                format!("invalid exact numeric literal: {error}"),
                span,
                Some("exact numeric literals must fit selene-db DECIMAL".into()),
            )
        })
}

fn classify_decimal_literal(text: &str) -> DecimalLiteralKind {
    let (_, unsigned) = split_text_sign(text);
    let has_suffix = unsigned.ends_with(['m', 'M']);
    let body = unsigned.strip_suffix(['m', 'M']).unwrap_or(unsigned);
    if contains_exponent(body) {
        DecimalLiteralKind::ScientificWithSuffix
    } else if has_suffix {
        DecimalLiteralKind::CommonOrIntegerWithSuffix
    } else {
        DecimalLiteralKind::CommonWithoutSuffix
    }
}

fn classify_float_literal(text: &str) -> FloatLiteralKind {
    let (_, unsigned) = split_text_sign(text);
    let suffix = unsigned.as_bytes().last().copied();
    let body = unsigned
        .strip_suffix(['f', 'd', 'F', 'D'])
        .unwrap_or(unsigned);
    match (contains_exponent(body), suffix) {
        (true, Some(b'f' | b'F')) => FloatLiteralKind::ScientificWithFloatSuffix,
        (true, Some(b'd' | b'D')) => FloatLiteralKind::ScientificWithDoubleSuffix,
        (true, _) => FloatLiteralKind::ScientificWithoutSuffix,
        (false, Some(b'f' | b'F')) => FloatLiteralKind::CommonOrIntegerWithFloatSuffix,
        (false, Some(b'd' | b'D')) => FloatLiteralKind::CommonOrIntegerWithDoubleSuffix,
        (false, _) => FloatLiteralKind::ScientificWithoutSuffix,
    }
}

fn normalize_decimal_image(image: &str) -> String {
    if let Some(index) = image.find(['e', 'E']) {
        let (mantissa, exponent) = image.split_at(index);
        format!("{}{}", normalize_decimal_mantissa(mantissa), exponent)
    } else {
        normalize_decimal_mantissa(image)
    }
}

fn normalize_decimal_mantissa(mantissa: &str) -> String {
    let (sign, unsigned) = split_text_sign(mantissa);
    let unsigned = if let Some(rest) = unsigned.strip_prefix('.') {
        format!("0.{rest}")
    } else if let Some(rest) = unsigned.strip_suffix('.') {
        rest.to_owned()
    } else {
        unsigned.to_owned()
    };
    format!("{sign}{unsigned}")
}

fn normalize_float_image(image: &str) -> String {
    if let Some(index) = image.find(['e', 'E']) {
        let (mantissa, exponent) = image.split_at(index);
        format!("{}{}", normalize_float_mantissa(mantissa), exponent)
    } else {
        normalize_float_mantissa(image)
    }
}

fn normalize_float_mantissa(mantissa: &str) -> String {
    let (sign, unsigned) = split_text_sign(mantissa);
    let unsigned = if let Some(rest) = unsigned.strip_prefix('.') {
        format!("0.{rest}")
    } else if let Some(rest) = unsigned.strip_suffix('.') {
        format!("{rest}.0")
    } else {
        unsigned.to_owned()
    };
    format!("{sign}{unsigned}")
}

fn split_text_sign(text: &str) -> (&str, &str) {
    if let Some(rest) = text.strip_prefix('-') {
        ("-", rest)
    } else if let Some(rest) = text.strip_prefix('+') {
        ("", rest)
    } else {
        ("", text)
    }
}

fn contains_exponent(image: &str) -> bool {
    image
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'e' | b'E'))
}

fn parse_byte_string_lit(text: &str, span: SourceSpan) -> Result<Literal, ParserError> {
    let Some(body) = text.strip_prefix('X').or_else(|| text.strip_prefix('x')) else {
        return Err(ParserError::syntax(
            "byte string literal is missing X prefix",
            span,
            None,
        ));
    };
    let mut in_chunk = false;
    let mut digits = Vec::with_capacity(body.len());
    for ch in body.chars() {
        match ch {
            '\'' => in_chunk = !in_chunk,
            ' ' if in_chunk => {}
            value if in_chunk && value.is_ascii_hexdigit() => digits.push(value),
            value if !in_chunk && value.is_ascii_whitespace() => {}
            _ => {
                return Err(ParserError::syntax(
                    "invalid byte string literal",
                    span,
                    Some("byte string literals use hexadecimal digit pairs".into()),
                ));
            }
        }
    }
    if in_chunk {
        return Err(ParserError::syntax(
            "unterminated byte string literal chunk",
            span,
            None,
        ));
    }
    if digits.len() % 2 != 0 {
        return Err(ParserError::syntax(
            "byte string literal has an odd number of hexadecimal digits",
            span,
            Some("use two hexadecimal digits per byte".into()),
        ));
    }

    let mut bytes = Vec::with_capacity(digits.len() / 2);
    for pair in digits.chunks_exact(2) {
        let high = hex_value(pair[0], span)?;
        let low = hex_value(pair[1], span)?;
        bytes.push((high << 4) | low);
    }
    Ok(Literal::Bytes(
        Arc::<[u8]>::from(bytes.into_boxed_slice()),
        span,
    ))
}

fn hex_value(ch: char, span: SourceSpan) -> Result<u8, ParserError> {
    ch.to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
        .ok_or_else(|| ParserError::syntax("invalid byte string hexadecimal digit", span, None))
}

fn validate_underscores(text: &str, span: SourceSpan) -> Result<(), ParserError> {
    let mut prev_underscore = false;
    for &byte in text.as_bytes() {
        if byte == b'_' {
            if prev_underscore {
                return Err(ParserError::syntax(
                    "numeric literal contains consecutive underscores",
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
            "numeric literal cannot end with an underscore",
            span,
            Some("remove the trailing `_`".into()),
        ));
    }
    Ok(())
}

fn parse_string(text: &str, span: SourceSpan) -> Result<Literal, ParserError> {
    let parsed = parse_character_string(text, span)?;
    let db_string_value = db_string_from_owned(parsed.value, span, "string literal")?;
    Ok(Literal::String(db_string_value, span, parsed.kind))
}

fn temporal_text(pair: Pair<'_, Rule>) -> Result<ParsedCharacterString, ParserError> {
    let string_pair = first_child(pair)?;
    parse_character_string(string_pair.as_str(), span(&string_pair))
}

struct ParsedCharacterString {
    value: String,
    kind: CharacterStringLiteralKind,
}

fn parse_character_string(
    text: &str,
    span: SourceSpan,
) -> Result<ParsedCharacterString, ParserError> {
    if let Some(quoted) = text.strip_prefix('@') {
        if let Some(inner) = quoted
            .strip_prefix('\'')
            .and_then(|value| value.strip_suffix('\''))
        {
            return Ok(ParsedCharacterString {
                value: inner.to_owned(),
                kind: CharacterStringLiteralKind::NoEscape,
            });
        }
        if let Some(inner) = quoted
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
        {
            return Ok(ParsedCharacterString {
                value: inner.to_owned(),
                kind: CharacterStringLiteralKind::NoEscape,
            });
        }
    }
    if let Some(inner) = text
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        return Ok(ParsedCharacterString {
            value: decode_quoted(inner, '\'', span)?,
            kind: CharacterStringLiteralKind::Escaped,
        });
    }
    if let Some(inner) = text
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        return Ok(ParsedCharacterString {
            value: decode_quoted(inner, '"', span)?,
            kind: CharacterStringLiteralKind::Escaped,
        });
    }
    Err(ParserError::syntax(
        "string literal is missing quotes",
        span,
        None,
    ))
}

fn decode_quoted(inner: &str, delimiter: char, span: SourceSpan) -> Result<String, ParserError> {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == delimiter && chars.peek() == Some(&delimiter) {
            chars.next();
            out.push(delimiter);
        } else {
            match ch {
                '\\' => out.push(decode_escape(&mut chars, span)?),
                _ => out.push(ch),
            }
        }
    }

    Ok(out)
}

fn temporal_message(message: impl Into<String>, span: SourceSpan) -> ParserError {
    ParserError::syntax_with_status(GqlStatus::INVALID_DATETIME_FORMAT, message, span, None)
}

fn duration_message(message: impl Into<String>, span: SourceSpan) -> ParserError {
    ParserError::syntax_with_status(GqlStatus::INVALID_DURATION_FORMAT, message, span, None)
}

fn decode_escape(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    span: SourceSpan,
) -> Result<char, ParserError> {
    let Some(escape) = chars.next() else {
        return Err(ParserError::syntax(
            "unterminated string escape",
            span,
            None,
        ));
    };
    match escape {
        'n' => Ok('\n'),
        'r' => Ok('\r'),
        't' => Ok('\t'),
        '\\' => Ok('\\'),
        '\'' => Ok('\''),
        '"' => Ok('"'),
        '`' => Ok('`'),
        'b' => Ok('\u{0008}'),
        'f' => Ok('\u{000c}'),
        'u' => decode_unicode_escape(chars, 4, span),
        'U' => decode_unicode_escape(chars, 8, span),
        _ => Err(ParserError::syntax("unknown string escape", span, None)),
    }
}

fn decode_unicode_escape(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    digits: usize,
    span: SourceSpan,
) -> Result<char, ParserError> {
    let mut value = 0_u32;
    for _ in 0..digits {
        let Some(ch) = chars.next() else {
            return Err(ParserError::syntax(
                "unterminated unicode escape",
                span,
                None,
            ));
        };
        let Some(digit) = ch.to_digit(16) else {
            return Err(ParserError::syntax("invalid unicode escape", span, None));
        };
        value = (value << 4) | digit;
    }
    char::from_u32(value).ok_or_else(|| ParserError::syntax("invalid unicode scalar", span, None))
}
