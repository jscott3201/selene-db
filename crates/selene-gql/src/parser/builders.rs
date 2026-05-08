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
    intern(prop_ident.as_str()).map_err(|error| {
        ParserError::syntax(
            format!("could not intern alias: {error}"),
            SourceSpan::from_pest(prop_ident.as_span()),
            Some("identifier interning cap may be exhausted".into()),
        )
    })
}

fn build_value_expr(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let span = SourceSpan::from_pest(pair.as_span());
    match pair.as_rule() {
        Rule::literal => build_literal(pair).map(ValueExpr::Literal),
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
        | Rule::unary
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
    let mut normalized = text.replace('_', "");
    if matches!(normalized.as_bytes().last(), Some(b'f' | b'd')) {
        normalized.pop();
    }
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
