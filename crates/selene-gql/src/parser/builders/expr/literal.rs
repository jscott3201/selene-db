//! Literal expression builders.

use pest::iterators::Pair;
use selene_core::IStr;

use crate::{
    GqlStatus,
    ast::{Literal, SourceSpan, ValueExpr},
    error::ParserError,
};

use super::{Rule, build_value_expr};
use crate::parser::builders::{first_child, intern_str, not_implemented, span};

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

pub(super) fn parse_string_pair(pair: Pair<'_, Rule>) -> Result<IStr, ParserError> {
    let Literal::String(value, _) = parse_string(pair.as_str(), span(&pair))? else {
        unreachable!("parse_string returns a string literal");
    };
    Ok(value)
}

/// Decode a `string_lit` pair into its raw (unquoted, unescaped) text.
///
/// Used by surfaces that need the decoded string value rather than an interned
/// literal — for example the `SESSION SET TIME ZONE '<region>'` time-zone
/// string (ISO/IEC 39075:2024 section 7.1).
pub(super) fn decode_string_text(pair: &Pair<'_, Rule>) -> Result<String, ParserError> {
    parse_string_text(pair.as_str(), span(pair))
}

pub(super) fn with_numeric_span(value: ValueExpr, source_span: SourceSpan) -> ValueExpr {
    match value {
        ValueExpr::Literal(Literal::Integer(value, _)) => {
            ValueExpr::Literal(Literal::Integer(value, source_span))
        }
        ValueExpr::Literal(Literal::Float(value, _)) => {
            ValueExpr::Literal(Literal::Float(value, source_span))
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
        Rule::float_lit => parse_f64(child.as_str(), child_span),
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
    let value = parse_string_text(string_pair.as_str(), span(&string_pair))?;
    uuid::Uuid::parse_str(&value)
        .map(|uuid| Literal::Uuid(uuid, source_span))
        .map_err(|error| {
            ParserError::syntax(format!("invalid UUID literal: {error}"), source_span, None)
        })
}

fn parse_date_lit(pair: Pair<'_, Rule>, source_span: SourceSpan) -> Result<Literal, ParserError> {
    let value = temporal_text(pair)?;
    value
        .parse::<jiff::civil::Date>()
        .map(|date| Literal::Date(date, source_span))
        .map_err(|error| temporal_error("DATE", error, source_span))
}

fn parse_local_datetime_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let value = temporal_text(pair)?;
    let pieces = parse_datetime_pieces(&value, "LOCAL DATETIME", source_span)?;
    reject_datetime_zone(&pieces, "LOCAL DATETIME", source_span)?;
    let time = pieces
        .time()
        .ok_or_else(|| temporal_message("LOCAL DATETIME literal requires a time", source_span))?;
    Ok(Literal::LocalDateTime(
        pieces.date().to_datetime(time),
        source_span,
    ))
}

fn parse_zoned_datetime_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let value = temporal_text(pair)?;
    parse_zoned_datetime_text(&value, "ZONED DATETIME", source_span)
        .map(|zoned| Literal::ZonedDateTime(Box::new(zoned), source_span))
}

fn parse_datetime_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let value = temporal_text(pair)?;
    let pieces = parse_datetime_pieces(&value, "DATETIME", source_span)?;
    if has_datetime_zone(&pieces, source_span)? {
        parse_zoned_datetime_text(&value, "DATETIME", source_span)
            .map(|zoned| Literal::ZonedDateTime(Box::new(zoned), source_span))
    } else {
        let time = pieces
            .time()
            .ok_or_else(|| temporal_message("DATETIME literal requires a time", source_span))?;
        Ok(Literal::LocalDateTime(
            pieces.date().to_datetime(time),
            source_span,
        ))
    }
}

fn parse_local_time_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let value = temporal_text(pair)?;
    if time_has_zone_designator(&value) {
        return Err(temporal_message(
            "LOCAL TIME literal must not include a time zone displacement",
            source_span,
        ));
    }
    value
        .parse::<jiff::civil::Time>()
        .map(|time| Literal::LocalTime(time, source_span))
        .map_err(|error| temporal_error("LOCAL TIME", error, source_span))
}

fn parse_zoned_time_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let value = temporal_text(pair)?;
    parse_zoned_time_text(&value, source_span)
        .map(|zoned| Literal::ZonedTime(Box::new(zoned), source_span))
}

fn parse_time_lit(pair: Pair<'_, Rule>, source_span: SourceSpan) -> Result<Literal, ParserError> {
    let value = temporal_text(pair)?;
    if time_has_zone_designator(&value) {
        parse_zoned_time_text(&value, source_span)
            .map(|zoned| Literal::ZonedTime(Box::new(zoned), source_span))
    } else {
        value
            .parse::<jiff::civil::Time>()
            .map(|time| Literal::LocalTime(time, source_span))
            .map_err(|error| temporal_error("TIME", error, source_span))
    }
}

fn parse_duration_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let value = temporal_text(pair)?;
    value
        .parse::<jiff::Span>()
        .map(|span| Literal::Duration(Box::new(span), source_span))
        .map_err(|error| temporal_error("DURATION", error, source_span))
}

fn parse_i64(text: &str, span: SourceSpan) -> Result<Literal, ParserError> {
    validate_underscores(text, span)?;
    let normalized = text.replace('_', "");
    normalized
        .parse::<i64>()
        .map(|value| Literal::Integer(value, span))
        .map_err(|error| {
            ParserError::syntax(
                format!("invalid integer literal: {error}"),
                span,
                Some("integer literals must fit in i64".into()),
            )
        })
}

fn parse_f64(text: &str, span: SourceSpan) -> Result<Literal, ParserError> {
    let trimmed = text.strip_suffix(['f', 'd', 'F', 'D']).unwrap_or(text);
    validate_underscores(trimmed, span)?;
    let normalized = trimmed.replace('_', "");
    normalized
        .parse::<f64>()
        .map(|value| Literal::Float(value, span))
        .map_err(|error| {
            ParserError::syntax(
                format!("invalid floating-point literal: {error}"),
                span,
                None,
            )
        })
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
    let value = parse_string_text(text, span)?;
    let interned = intern_str(&value, span, "string literal")?;
    Ok(Literal::String(interned, span))
}

fn temporal_text(pair: Pair<'_, Rule>) -> Result<String, ParserError> {
    let string_pair = first_child(pair)?;
    parse_string_text(string_pair.as_str(), span(&string_pair))
}

fn parse_zoned_datetime_text(
    text: &str,
    kind: &'static str,
    source_span: SourceSpan,
) -> Result<jiff::Zoned, ParserError> {
    let pieces = parse_datetime_pieces(text, kind, source_span)?;
    let time = pieces
        .time()
        .ok_or_else(|| temporal_message(format!("{kind} literal requires a time"), source_span))?;
    let zone = pieces
        .to_time_zone()
        .map_err(|error| temporal_error(kind, error, source_span))?
        .or_else(|| pieces.to_numeric_offset().map(jiff::tz::TimeZone::fixed))
        .ok_or_else(|| {
            temporal_message(
                format!("{kind} literal requires a time zone displacement"),
                source_span,
            )
        })?;
    pieces
        .date()
        .to_datetime(time)
        .to_zoned(zone)
        .map_err(|error| temporal_error(kind, error, source_span))
}

fn parse_zoned_time_text(text: &str, source_span: SourceSpan) -> Result<jiff::Zoned, ParserError> {
    if !time_has_zone_designator(text) {
        return Err(temporal_message(
            "ZONED TIME literal requires a time zone displacement",
            source_span,
        ));
    }
    let anchored = format!("1970-01-01T{text}");
    parse_zoned_datetime_text(&anchored, "ZONED TIME", source_span)
}

fn parse_datetime_pieces<'a>(
    text: &'a str,
    kind: &'static str,
    source_span: SourceSpan,
) -> Result<jiff::fmt::temporal::Pieces<'a>, ParserError> {
    jiff::fmt::temporal::DateTimeParser::new()
        .parse_pieces(text)
        .map_err(|error| temporal_error(kind, error, source_span))
}

fn reject_datetime_zone(
    pieces: &jiff::fmt::temporal::Pieces<'_>,
    kind: &'static str,
    source_span: SourceSpan,
) -> Result<(), ParserError> {
    if has_datetime_zone(pieces, source_span)? {
        return Err(temporal_message(
            format!("{kind} literal must not include a time zone displacement"),
            source_span,
        ));
    }
    Ok(())
}

fn has_datetime_zone(
    pieces: &jiff::fmt::temporal::Pieces<'_>,
    source_span: SourceSpan,
) -> Result<bool, ParserError> {
    Ok(pieces.to_numeric_offset().is_some()
        || pieces
            .to_time_zone()
            .map_err(|error| temporal_error("DATETIME", error, source_span))?
            .is_some())
}

fn time_has_zone_designator(text: &str) -> bool {
    text.ends_with(['Z', 'z']) || text.contains('[') || text.bytes().any(|b| b == b'+' || b == b'-')
}

fn parse_string_text(text: &str, span: SourceSpan) -> Result<String, ParserError> {
    let inner = text
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .ok_or_else(|| ParserError::syntax("string literal is missing quotes", span, None))?;
    decode_single_quoted(inner, span)
}

fn decode_single_quoted(inner: &str, span: SourceSpan) -> Result<String, ParserError> {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if chars.peek() == Some(&'\'') => {
                chars.next();
                out.push('\'');
            }
            '\\' => out.push(decode_escape(&mut chars, span)?),
            _ => out.push(ch),
        }
    }

    Ok(out)
}

fn temporal_error(
    kind: &'static str,
    error: impl std::fmt::Display,
    span: SourceSpan,
) -> ParserError {
    temporal_message(format!("invalid {kind} literal: {error}"), span)
}

fn temporal_message(message: impl Into<String>, span: SourceSpan) -> ParserError {
    ParserError::syntax_with_status(GqlStatus::INVALID_DATETIME_FORMAT, message, span, None)
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
