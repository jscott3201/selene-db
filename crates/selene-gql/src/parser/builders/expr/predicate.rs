//! Predicate expression builders.

use pest::iterators::Pair;

use crate::{
    ast::{BinaryOp, GqlType, IsCheckKind, NormalForm, SourceSpan, TruthValue, ValueExpr},
    error::ParserError,
};

use super::{Rule, build_value_expr, literal};
use crate::parser::builders::{not_implemented, pattern, span, unexpected_pair};

pub(super) fn apply_is_suffix(
    operand: ValueExpr,
    suffix: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<ValueExpr, ParserError> {
    debug_assert_eq!(suffix.as_rule(), Rule::is_suffix);
    let text = suffix.as_str().to_ascii_uppercase();
    let negated = has_word(&text, "NOT");

    if has_word(&text, "IN") {
        let list_pair = suffix
            .into_inner()
            .find(|child| child.as_rule() == Rule::list_lit)
            .ok_or_else(|| {
                ParserError::syntax("IN predicate is missing list", source_span, None)
            })?;
        return Ok(ValueExpr::InList {
            operand: Box::new(operand),
            list: literal::build_list_items(list_pair)?,
            negated,
            span: source_span,
        });
    }

    if has_word(&text, "LIKE") {
        let pattern_pair = suffix
            .into_inner()
            .find(|child| child.as_rule() == Rule::addition)
            .ok_or_else(|| {
                ParserError::syntax("LIKE predicate is missing pattern", source_span, None)
            })?;
        return Ok(ValueExpr::Like {
            operand: Box::new(operand),
            pattern: Box::new(build_value_expr(pattern_pair)?),
            negated,
            span: source_span,
        });
    }

    if has_word(&text, "BETWEEN") {
        let bounds = suffix
            .into_inner()
            .filter(|child| child.as_rule() == Rule::addition)
            .map(build_value_expr)
            .collect::<Result<Vec<_>, _>>()?;
        if bounds.len() != 2 {
            return Err(ParserError::syntax(
                "BETWEEN predicate requires two bounds",
                source_span,
                None,
            ));
        }
        return Ok(ValueExpr::Between {
            operand: Box::new(operand),
            low: Box::new(bounds[0].clone()),
            high: Box::new(bounds[1].clone()),
            negated,
            span: source_span,
        });
    }

    if text.starts_with("STARTS WITH")
        || text.starts_with("ENDS WITH")
        || text.starts_with("CONTAINS")
    {
        return build_string_match(operand, suffix, &text, source_span);
    }

    Ok(ValueExpr::IsCheck {
        operand: Box::new(operand),
        kind: build_is_kind(suffix, &text, source_span)?,
        negated,
        span: source_span,
    })
}

fn build_string_match(
    operand: ValueExpr,
    suffix: Pair<'_, Rule>,
    text: &str,
    source_span: SourceSpan,
) -> Result<ValueExpr, ParserError> {
    let op = if text.starts_with("STARTS WITH") {
        BinaryOp::StartsWith
    } else if text.starts_with("ENDS WITH") {
        BinaryOp::EndsWith
    } else {
        BinaryOp::Contains
    };
    let rhs_pair = suffix
        .into_inner()
        .find(|child| child.as_rule() == Rule::comparison)
        .ok_or_else(|| {
            ParserError::syntax(
                "string-match predicate is missing operand",
                source_span,
                None,
            )
        })?;
    Ok(ValueExpr::BinaryOp {
        op,
        lhs: Box::new(operand),
        rhs: Box::new(build_value_expr(rhs_pair)?),
        span: source_span,
    })
}

fn build_is_kind(
    suffix: Pair<'_, Rule>,
    text: &str,
    source_span: SourceSpan,
) -> Result<IsCheckKind, ParserError> {
    if has_word(text, "NULL") {
        Ok(IsCheckKind::Null)
    } else if has_word(text, "LABELED") {
        let label_pair = suffix
            .into_inner()
            .find(|child| child.as_rule() == Rule::label_expr)
            .ok_or_else(|| {
                ParserError::syntax("IS LABELED is missing label expression", source_span, None)
            })?;
        Ok(IsCheckKind::Labeled(pattern::build_label_expr(label_pair)?))
    } else if text.contains("SOURCE OF") || text.contains("DESTINATION OF") {
        build_endpoint_kind(suffix, text, source_span)
    } else if has_word(text, "DIRECTED") {
        Ok(IsCheckKind::Directed)
    } else if has_word(text, "NORMALIZED") {
        Ok(IsCheckKind::Normalized(normal_form(text)))
    } else if has_word(text, "TRUE") {
        Ok(IsCheckKind::TruthValue(TruthValue::True))
    } else if has_word(text, "FALSE") {
        Ok(IsCheckKind::TruthValue(TruthValue::False))
    } else if has_word(text, "UNKNOWN") {
        Ok(IsCheckKind::TruthValue(TruthValue::Unknown))
    } else if has_word(text, "TYPED") {
        let type_pair = suffix
            .into_inner()
            .find(|child| child.as_rule() == Rule::type_name)
            .ok_or_else(|| ParserError::syntax("IS TYPED is missing type", source_span, None))?;
        Ok(IsCheckKind::Typed(build_type_name(type_pair)?))
    } else {
        Err(unexpected_pair(suffix, "unsupported IS predicate"))
    }
}

fn build_endpoint_kind(
    suffix: Pair<'_, Rule>,
    text: &str,
    source_span: SourceSpan,
) -> Result<IsCheckKind, ParserError> {
    let rhs_pair = suffix
        .into_inner()
        .find(|child| child.as_rule() == Rule::comparison)
        .ok_or_else(|| {
            ParserError::syntax(
                "IS endpoint predicate is missing expression",
                source_span,
                None,
            )
        })?;
    let rhs = Box::new(build_value_expr(rhs_pair)?);
    if text.contains("SOURCE OF") {
        Ok(IsCheckKind::SourceOf(rhs))
    } else {
        Ok(IsCheckKind::DestinationOf(rhs))
    }
}

pub(super) fn build_type_name(pair: Pair<'_, Rule>) -> Result<GqlType, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::type_name);
    let source_span = span(&pair);
    let text = pair.as_str().to_ascii_uppercase();
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.starts_with("LIST") {
        let inner = pair
            .into_inner()
            .find(|child| child.as_rule() == Rule::type_name)
            .ok_or_else(|| {
                ParserError::syntax("LIST type is missing element type", source_span, None)
            })?;
        return Ok(GqlType::List(Box::new(build_type_name(inner)?)));
    }

    match compact.as_str() {
        "BOOLEAN" | "BOOL" => Ok(GqlType::Boolean),
        "SIGNED INTEGER" | "INTEGER" | "INT" => Ok(GqlType::Integer),
        "INT8" => Ok(GqlType::Int8),
        "INT16" => Ok(GqlType::Int16),
        "INT32" => Ok(GqlType::Int32),
        "INT64" => Ok(GqlType::Int64),
        "INT128" => Ok(GqlType::Int128),
        "SMALLINT" => Ok(GqlType::SmallInt),
        "BIGINT" => Ok(GqlType::BigInt),
        "UINT" | "UINT64" => Ok(GqlType::Uint64),
        "UINT8" => Ok(GqlType::Uint8),
        "UINT16" => Ok(GqlType::Uint16),
        "UINT32" => Ok(GqlType::Uint32),
        "UINT128" => Ok(GqlType::Uint128),
        "DOUBLE" | "DOUBLE PRECISION" | "FLOAT" | "REAL" => Ok(GqlType::Float),
        "FLOAT32" => Ok(GqlType::Float32),
        "FLOAT64" => Ok(GqlType::Float64),
        "STRING" | "VARCHAR" | "UUID" => Ok(GqlType::String),
        "BYTES" | "BYTEA" => Ok(GqlType::Bytes),
        "ZONED DATETIME" => Ok(GqlType::ZonedDateTime),
        "LOCAL DATETIME" => Ok(GqlType::LocalDateTime),
        "ZONED TIME" => Ok(GqlType::ZonedTime),
        "LOCAL TIME" => Ok(GqlType::LocalTime),
        "DATE" => Ok(GqlType::Date),
        "DURATION" => Ok(GqlType::Duration),
        "PATH" => Ok(GqlType::Path),
        "NULL" => Ok(GqlType::Null),
        "NOTHING" => Ok(GqlType::Nothing),
        _ => Err(not_implemented(&pair, "this GQL type builder lands in M5b")),
    }
}

fn normal_form(text: &str) -> NormalForm {
    if has_word(text, "NFD") {
        NormalForm::Nfd
    } else if has_word(text, "NFKC") {
        NormalForm::Nfkc
    } else if has_word(text, "NFKD") {
        NormalForm::Nfkd
    } else {
        NormalForm::Nfc
    }
}

fn has_word(text: &str, word: &str) -> bool {
    text.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .any(|part| part == word)
}
