//! Pair-to-AST builders for the BRIEF-16 parser surface.

use pest::iterators::Pair;
use selene_core::intern;

use crate::{
    ast::{Literal, ReturnItem, ReturnStatement, SourceSpan, Statement, ValueExpr},
    error::ParserError,
};

use super::Rule;

pub(crate) fn build_statement(program_pair: Pair<'_, Rule>) -> Result<Statement, ParserError> {
    let return_pair = unwrap_single_return(program_pair)?;
    build_return_statement(return_pair).map(Statement::Return)
}

fn unwrap_single_return(pair: Pair<'_, Rule>) -> Result<Pair<'_, Rule>, ParserError> {
    match pair.as_rule() {
        Rule::return_stmt => Ok(pair),
        Rule::gql_program | Rule::query_pipeline | Rule::pipeline_statement => {
            let span = SourceSpan::from_pest(pair.as_span());
            let mut children = pair
                .into_inner()
                .filter(|child| child.as_rule() != Rule::EOI);
            let Some(child) = children.next() else {
                return Err(ParserError::empty_program());
            };
            if children.next().is_some() {
                return Err(ParserError::unsupported_bootstrap(span));
            }
            unwrap_single_return(child)
        }
        _ => Err(ParserError::unsupported_bootstrap(SourceSpan::from_pest(
            pair.as_span(),
        ))),
    }
}

fn build_return_statement(pair: Pair<'_, Rule>) -> Result<ReturnStatement, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::return_stmt);
    let span = SourceSpan::from_pest(pair.as_span());
    let mut items = Vec::new();

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::projection_list => items.extend(build_projection_list(child)?),
            Rule::distinct_kw
            | Rule::return_star
            | Rule::no_bindings
            | Rule::group_by_clause
            | Rule::having_clause => {
                return Err(ParserError::unsupported_bootstrap(SourceSpan::from_pest(
                    child.as_span(),
                )));
            }
            _ => {
                return Err(unexpected_pair(child, "expected a RETURN projection list"));
            }
        }
    }

    if items.is_empty() {
        return Err(ParserError::syntax(
            "RETURN requires at least one projection item",
            span,
            None,
        ));
    }

    Ok(ReturnStatement { items, span })
}

fn build_projection_list(pair: Pair<'_, Rule>) -> Result<Vec<ReturnItem>, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::projection_list);
    pair.into_inner().map(build_return_item).collect()
}

fn build_return_item(pair: Pair<'_, Rule>) -> Result<ReturnItem, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::projection);
    let span = SourceSpan::from_pest(pair.as_span());
    let mut expr = None;
    let mut alias = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::expr => expr = Some(build_value_expr(child)?),
            Rule::alias => alias = Some(build_alias(child)?),
            _ => return Err(unexpected_pair(child, "expected expression or AS alias")),
        }
    }

    let expr = expr
        .ok_or_else(|| ParserError::syntax("RETURN item is missing an expression", span, None))?;
    Ok(ReturnItem { expr, alias, span })
}

fn build_alias(pair: Pair<'_, Rule>) -> Result<selene_core::IStr, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::alias);
    let span = SourceSpan::from_pest(pair.as_span());
    let prop_ident = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::prop_ident)
        .ok_or_else(|| ParserError::syntax("AS alias is missing an identifier", span, None))?;
    let ident_span = SourceSpan::from_pest(prop_ident.as_span());
    let canonical = decode_prop_ident(prop_ident.as_str());
    intern(&canonical).map_err(|error| {
        ParserError::syntax(
            format!("could not intern alias: {error}"),
            ident_span,
            Some("identifier interning cap may be exhausted".into()),
        )
    })
}

fn decode_prop_ident(text: &str) -> String {
    if let Some(inner) = text.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        inner.replace("\"\"", "\"")
    } else if let Some(inner) = text.strip_prefix('`').and_then(|s| s.strip_suffix('`')) {
        inner.to_owned()
    } else {
        text.to_owned()
    }
}

fn build_value_expr(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let span = SourceSpan::from_pest(pair.as_span());
    match pair.as_rule() {
        Rule::literal => build_literal(pair).map(ValueExpr::Literal),
        Rule::unary => build_unary(pair, span),
        Rule::expr
        | Rule::or_expr
        | Rule::xor_expr
        | Rule::and_expr
        | Rule::not_expr
        | Rule::is_expr
        | Rule::comparison
        | Rule::concat
        | Rule::addition
        | Rule::multiplication
        | Rule::postfix
        | Rule::primary => {
            let mut children = pair.into_inner();
            let Some(child) = children.next() else {
                return Err(ParserError::syntax("expression is empty", span, None));
            };
            if children.next().is_some() {
                return Err(ParserError::unsupported_bootstrap(span));
            }
            build_value_expr(child)
        }
        _ => Err(ParserError::unsupported_bootstrap(span)),
    }
}

fn build_unary(pair: Pair<'_, Rule>, outer_span: SourceSpan) -> Result<ValueExpr, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::unary);
    let mut children = pair.into_inner();
    let Some(first) = children.next() else {
        return Err(ParserError::syntax("expression is empty", outer_span, None));
    };

    if first.as_rule() != Rule::sign_op {
        if children.next().is_some() {
            return Err(ParserError::unsupported_bootstrap(outer_span));
        }
        return build_value_expr(first);
    }

    let is_negative = first.as_str() == "-";
    let Some(operand) = children.next() else {
        return Err(ParserError::syntax(
            "unary operator missing operand",
            outer_span,
            None,
        ));
    };
    if children.next().is_some() {
        return Err(ParserError::unsupported_bootstrap(outer_span));
    }

    let inner = build_value_expr(operand)?;
    apply_sign(is_negative, inner, outer_span)
}

fn apply_sign(
    is_negative: bool,
    value: ValueExpr,
    outer_span: SourceSpan,
) -> Result<ValueExpr, ParserError> {
    match value {
        ValueExpr::Literal(Literal::Integer(v, _)) => {
            let signed = if is_negative {
                v.checked_neg().ok_or_else(|| {
                    ParserError::syntax(
                        "integer literal overflows i64 after negation",
                        outer_span,
                        Some("the most-negative i64 cannot be written as -<unsigned>".into()),
                    )
                })?
            } else {
                v
            };
            Ok(ValueExpr::Literal(Literal::Integer(signed, outer_span)))
        }
        ValueExpr::Literal(Literal::Float(v, _)) => {
            let signed = if is_negative { -v } else { v };
            Ok(ValueExpr::Literal(Literal::Float(signed, outer_span)))
        }
        _ => Err(ParserError::unsupported_bootstrap(outer_span)),
    }
}

fn build_literal(pair: Pair<'_, Rule>) -> Result<Literal, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::literal);
    let span = SourceSpan::from_pest(pair.as_span());
    let mut children = pair.into_inner();
    let Some(child) = children.next() else {
        return Err(ParserError::syntax("literal is empty", span, None));
    };
    if children.next().is_some() {
        return Err(ParserError::syntax(
            "literal has multiple children",
            span,
            None,
        ));
    }

    let child_span = SourceSpan::from_pest(child.as_span());
    match child.as_rule() {
        Rule::null_lit => Ok(Literal::Null(child_span)),
        Rule::bool_lit => Ok(Literal::Bool(
            child.as_str().eq_ignore_ascii_case("true"),
            child_span,
        )),
        Rule::int_lit => parse_i64(child.as_str(), child_span),
        Rule::float_lit => parse_f64(child.as_str(), child_span),
        Rule::string_lit => parse_string(child.as_str(), child_span),
        _ => Err(ParserError::unsupported_bootstrap(child_span)),
    }
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
                Some("integer literals in BRIEF-16 must fit in i64".into()),
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

/// Reject `__` runs and trailing `_` in a numeric literal.
///
/// Aligns selene-db's underscore policy with Rust's: separators may appear
/// between digits but never consecutively and never at the trailing edge.
/// Leading underscores cannot occur because the grammar requires the literal
/// to start with a digit (or sign + digit, the sign already extracted).
fn validate_underscores(text: &str, span: SourceSpan) -> Result<(), ParserError> {
    let bytes = text.as_bytes();
    let mut prev_underscore = false;
    for &b in bytes {
        if b == b'_' {
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
    let inner = text
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .ok_or_else(|| ParserError::syntax("string literal is missing quotes", span, None))?;
    let value = decode_single_quoted(inner, span)?;
    let interned = intern(&value).map_err(|error| {
        ParserError::syntax(
            format!("could not intern string literal: {error}"),
            span,
            Some("string literal interning cap may be exhausted".into()),
        )
    })?;
    Ok(Literal::String(interned, span))
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

fn unexpected_pair(pair: Pair<'_, Rule>, message: &'static str) -> ParserError {
    ParserError::syntax(message, SourceSpan::from_pest(pair.as_span()), None)
}
