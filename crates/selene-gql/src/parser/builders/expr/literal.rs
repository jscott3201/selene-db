//! Literal expression builders.

use pest::iterators::Pair;
use selene_core::DbString;

use crate::{
    GqlStatus,
    ast::{Literal, SourceSpan, ValueExpr},
    error::ParserError,
    temporal_parse::{self, ParsedDateTime, ParsedTime},
};

use super::{Rule, build_value_expr};
use crate::parser::builders::{db_string_from_str, first_child, not_implemented, span};

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

pub(super) fn parse_string_pair(pair: Pair<'_, Rule>) -> Result<DbString, ParserError> {
    let Literal::String(value, _) = parse_string(pair.as_str(), span(&pair))? else {
        unreachable!("parse_string returns a string literal");
    };
    Ok(value)
}

/// Decode a `string_lit` pair into its raw (unquoted, unescaped) text.
///
/// Used by surfaces that need the decoded string value rather than a `DbString`
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
    temporal_parse::parse_date(&value)
        .map(|date| Literal::Date(date, source_span))
        .map_err(|error| temporal_message(error, source_span))
}

fn parse_local_datetime_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let value = temporal_text(pair)?;
    temporal_parse::parse_local_datetime(&value)
        .map(|datetime| Literal::LocalDateTime(datetime, source_span))
        .map_err(|error| temporal_message(error, source_span))
}

fn parse_zoned_datetime_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let value = temporal_text(pair)?;
    temporal_parse::parse_zoned_datetime(&value)
        .map(|zoned| Literal::ZonedDateTime(Box::new(zoned), source_span))
        .map_err(|error| temporal_message(error, source_span))
}

fn parse_datetime_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let value = temporal_text(pair)?;
    temporal_parse::parse_datetime(&value)
        .map(|parsed| match parsed {
            ParsedDateTime::Zoned(zoned) => Literal::ZonedDateTime(Box::new(zoned), source_span),
            ParsedDateTime::Local(datetime) => Literal::LocalDateTime(datetime, source_span),
        })
        .map_err(|error| temporal_message(error, source_span))
}

fn parse_local_time_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let value = temporal_text(pair)?;
    temporal_parse::parse_local_time(&value)
        .map(|time| Literal::LocalTime(time, source_span))
        .map_err(|error| temporal_message(error, source_span))
}

fn parse_zoned_time_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let value = temporal_text(pair)?;
    temporal_parse::parse_zoned_time(&value)
        .map(|zoned| Literal::ZonedTime(Box::new(zoned), source_span))
        .map_err(|error| temporal_message(error, source_span))
}

fn parse_time_lit(pair: Pair<'_, Rule>, source_span: SourceSpan) -> Result<Literal, ParserError> {
    let value = temporal_text(pair)?;
    temporal_parse::parse_time(&value)
        .map(|parsed| match parsed {
            ParsedTime::Zoned(zoned) => Literal::ZonedTime(Box::new(zoned), source_span),
            ParsedTime::Local(time) => Literal::LocalTime(time, source_span),
        })
        .map_err(|error| temporal_message(error, source_span))
}

fn parse_duration_lit(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<Literal, ParserError> {
    let value = temporal_text(pair)?;
    temporal_parse::parse_duration(&value)
        .map(|span| Literal::Duration(Box::new(span), source_span))
        .map_err(|error| duration_message(error, source_span))
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
    let db_string_value = db_string_from_str(&value, span, "string literal")?;
    Ok(Literal::String(db_string_value, span))
}

fn temporal_text(pair: Pair<'_, Rule>) -> Result<String, ParserError> {
    let string_pair = first_child(pair)?;
    parse_string_text(string_pair.as_str(), span(&string_pair))
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
