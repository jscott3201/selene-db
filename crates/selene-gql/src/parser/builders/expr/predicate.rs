//! Predicate expression builders.

use pest::iterators::Pair;
use selene_core::feature_register::FeatureId;

use crate::{
    ast::{BinaryOp, GqlType, IsCheckKind, NormalForm, SourceSpan, TruthValue, ValueExpr},
    error::ParserError,
    parser::{MAX_NESTING_DEPTH, budget::InternerBudget},
};

use super::{Rule, build_value_expr, literal};
use crate::parser::builders::{not_implemented, pattern, span};

pub(super) fn apply_is_suffix(
    operand: ValueExpr,
    suffix: Pair<'_, Rule>,
    source_span: SourceSpan,
    budget: &mut InternerBudget,
) -> Result<ValueExpr, ParserError> {
    debug_assert_eq!(suffix.as_rule(), Rule::is_suffix);
    let children: Vec<_> = suffix.into_inner().collect();
    // Negation is taken from the parsed `not_kw` token, not by scanning the
    // source text. Substring scans were misleading when an operand contained
    // a quoted identifier with `NOT` in it (e.g. `IS LABELED :"NOT"`).
    let negated = children.iter().any(|child| child.as_rule() == Rule::not_kw);
    dispatch_is_suffix(operand, &children, negated, source_span, budget)
}

fn dispatch_is_suffix(
    operand: ValueExpr,
    children: &[Pair<'_, Rule>],
    negated: bool,
    source_span: SourceSpan,
    budget: &mut InternerBudget,
) -> Result<ValueExpr, ParserError> {
    if let Some(string_op) = children
        .iter()
        .find(|child| child.as_rule() == Rule::string_match_op)
    {
        return build_string_match(operand, string_op, children, source_span, budget);
    }

    if children.iter().any(|child| child.as_rule() == Rule::in_kw) {
        let list_pair = find_child(children, Rule::list_lit, "IN predicate is missing list")?;
        return Ok(ValueExpr::InList {
            operand: Box::new(operand),
            list: literal::build_list_items(list_pair, budget)?,
            negated,
            span: source_span,
        });
    }

    if children
        .iter()
        .any(|child| child.as_rule() == Rule::like_kw)
    {
        let pattern_pair = find_child(
            children,
            Rule::addition,
            "LIKE predicate is missing pattern",
        )?;
        return Ok(ValueExpr::Like {
            operand: Box::new(operand),
            pattern: Box::new(build_value_expr(pattern_pair, budget)?),
            negated,
            span: source_span,
        });
    }

    if children
        .iter()
        .any(|child| child.as_rule() == Rule::between_kw)
    {
        let bounds = children
            .iter()
            .filter(|child| child.as_rule() == Rule::addition)
            .cloned()
            .map(|child| build_value_expr(child, budget))
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

    Ok(ValueExpr::IsCheck {
        operand: Box::new(operand),
        kind: build_is_kind(children, source_span, budget)?,
        negated,
        span: source_span,
    })
}

fn build_string_match(
    operand: ValueExpr,
    string_op: &Pair<'_, Rule>,
    children: &[Pair<'_, Rule>],
    source_span: SourceSpan,
    budget: &mut InternerBudget,
) -> Result<ValueExpr, ParserError> {
    let upper = string_op.as_str().to_ascii_uppercase();
    let op = if upper.starts_with("STARTS") {
        BinaryOp::StartsWith
    } else if upper.starts_with("ENDS") {
        BinaryOp::EndsWith
    } else {
        BinaryOp::Contains
    };
    let rhs_pair = find_child(
        children,
        Rule::comparison,
        "string-match predicate is missing operand",
    )?;
    Ok(ValueExpr::BinaryOp {
        op,
        lhs: Box::new(operand),
        rhs: Box::new(build_value_expr(rhs_pair, budget)?),
        span: source_span,
    })
}

fn build_is_kind(
    children: &[Pair<'_, Rule>],
    source_span: SourceSpan,
    budget: &mut InternerBudget,
) -> Result<IsCheckKind, ParserError> {
    if children
        .iter()
        .any(|child| child.as_rule() == Rule::null_kw)
    {
        return Ok(IsCheckKind::Null);
    }
    if children
        .iter()
        .any(|child| child.as_rule() == Rule::labeled_kw)
    {
        let label_pair = find_child(
            children,
            Rule::label_expr,
            "IS LABELED is missing label expression",
        )?;
        return Ok(IsCheckKind::Labeled(pattern::build_label_expr(
            label_pair, budget,
        )?));
    }
    if children
        .iter()
        .any(|child| child.as_rule() == Rule::source_of_kw)
    {
        let rhs_pair = find_child(
            children,
            Rule::comparison,
            "IS SOURCE OF is missing expression",
        )?;
        return Ok(IsCheckKind::SourceOf(Box::new(build_value_expr(
            rhs_pair, budget,
        )?)));
    }
    if children
        .iter()
        .any(|child| child.as_rule() == Rule::destination_of_kw)
    {
        let rhs_pair = find_child(
            children,
            Rule::comparison,
            "IS DESTINATION OF is missing expression",
        )?;
        return Ok(IsCheckKind::DestinationOf(Box::new(build_value_expr(
            rhs_pair, budget,
        )?)));
    }
    if children
        .iter()
        .any(|child| child.as_rule() == Rule::directed_kw)
    {
        return Ok(IsCheckKind::Directed);
    }
    if children
        .iter()
        .any(|child| child.as_rule() == Rule::normalized_kw)
    {
        let form = children
            .iter()
            .find(|child| child.as_rule() == Rule::normal_form)
            .map(|child| match child.as_str().to_ascii_uppercase().as_str() {
                "NFD" => NormalForm::Nfd,
                "NFKC" => NormalForm::Nfkc,
                "NFKD" => NormalForm::Nfkd,
                _ => NormalForm::Nfc,
            })
            .unwrap_or(NormalForm::Nfc);
        return Ok(IsCheckKind::Normalized(form));
    }
    if let Some(truth) = children
        .iter()
        .find(|child| child.as_rule() == Rule::truth_value)
    {
        let value = match truth.as_str().to_ascii_uppercase().as_str() {
            "TRUE" => TruthValue::True,
            "FALSE" => TruthValue::False,
            _ => TruthValue::Unknown,
        };
        return Ok(IsCheckKind::TruthValue(value));
    }
    if children
        .iter()
        .any(|child| child.as_rule() == Rule::typed_kw)
    {
        let type_pair = find_child(children, Rule::type_name, "IS TYPED is missing type")?;
        return Ok(IsCheckKind::Typed(build_type_name(type_pair)?));
    }
    Err(ParserError::syntax(
        "unsupported IS predicate",
        source_span,
        None,
    ))
}

fn find_child<'a>(
    children: &'a [Pair<'a, Rule>],
    rule: Rule,
    missing: &'static str,
) -> Result<Pair<'a, Rule>, ParserError> {
    children
        .iter()
        .find(|child| child.as_rule() == rule)
        .cloned()
        .ok_or_else(|| ParserError::syntax(missing, SourceSpan::default(), None))
}

pub(super) fn build_type_name(pair: Pair<'_, Rule>) -> Result<GqlType, ParserError> {
    build_type_name_with_depth(pair, 0)
}

fn build_type_name_with_depth(pair: Pair<'_, Rule>, depth: u32) -> Result<GqlType, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::type_name);
    let source_span = span(&pair);
    if depth > MAX_NESTING_DEPTH {
        return Err(ParserError::NestingLimitExceeded {
            limit: MAX_NESTING_DEPTH,
            span: source_span,
        });
    }
    let text = pair.as_str().to_ascii_uppercase();
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact == "REAL" {
        return Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV20,
            display_name: "Approximate value type: REAL",
            span: source_span,
            hint: "REAL type spelling is outside the selene-db v1.0 claim list; use FLOAT32 or FLOAT64",
        });
    }
    if compact == "FLOAT16" {
        return Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV20,
            display_name: "16 bit floating point numbers",
            span: source_span,
            hint: "FLOAT16 is outside the selene-db v1.0 claim list; use FLOAT32 or FLOAT64",
        });
    }
    if compact == "DOUBLE" || compact == "DOUBLE PRECISION" {
        return Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV23,
            display_name: "Floating point type name synonyms",
            span: source_span,
            hint: "DOUBLE spelling is outside the selene-db v1.0 claim list; use FLOAT64",
        });
    }
    if compact == "UINT256" {
        return Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV15,
            display_name: "256 bit unsigned integer numbers",
            span: source_span,
            hint: "UINT256 is outside the selene-db v1.0 claim list",
        });
    }
    if compact == "INT256" {
        return Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV16,
            display_name: "256 bit signed integer numbers",
            span: source_span,
            hint: "INT256 is outside the selene-db v1.0 claim list",
        });
    }
    if compact == "FLOAT128" {
        return Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV25,
            display_name: "128 bit floating point numbers",
            span: source_span,
            hint: "FLOAT128 is outside the selene-db v1.0 claim list",
        });
    }
    if compact == "FLOAT256" {
        return Err(ParserError::UnsupportedFeature {
            feature_id: FeatureId::GV26,
            display_name: "256 bit floating point numbers",
            span: source_span,
            hint: "FLOAT256 is outside the selene-db v1.0 claim list",
        });
    }
    if compact.starts_with("LIST") {
        let inner = pair
            .into_inner()
            .find(|child| child.as_rule() == Rule::type_name)
            .ok_or_else(|| {
                ParserError::syntax("LIST type is missing element type", source_span, None)
            })?;
        return Ok(GqlType::List(Box::new(build_type_name_with_depth(
            inner,
            depth + 1,
        )?)));
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
        "FLOAT" => Ok(GqlType::Float),
        "DECIMAL" | "DEC" => Ok(GqlType::Decimal),
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
        _ => Err(not_implemented(
            &pair,
            "this GQL type constructor is not yet supported in v1.0",
        )),
    }
}
