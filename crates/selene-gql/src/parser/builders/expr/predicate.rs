//! Predicate expression builders.

use pest::iterators::Pair;

use crate::{
    ast::{BinaryOp, IsCheckKind, NormalForm, SourceSpan, TruthValue, ValueExpr},
    error::ParserError,
};

use super::{Rule, build_value_expr, literal, type_name::build_type_name};
use crate::parser::builders::pattern;

pub(super) fn apply_is_suffix(
    operand: ValueExpr,
    suffix: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<ValueExpr, ParserError> {
    debug_assert_eq!(suffix.as_rule(), Rule::is_suffix);
    let children: Vec<_> = suffix.into_inner().collect();
    // Negation is taken from the parsed `not_kw` token, not by scanning the
    // source text. Substring scans were misleading when an operand contained
    // a quoted identifier with `NOT` in it (e.g. `IS LABELED :"NOT"`).
    let negated = children.iter().any(|child| child.as_rule() == Rule::not_kw);
    dispatch_is_suffix(operand, &children, negated, source_span)
}

fn dispatch_is_suffix(
    operand: ValueExpr,
    children: &[Pair<'_, Rule>],
    negated: bool,
    source_span: SourceSpan,
) -> Result<ValueExpr, ParserError> {
    if let Some(string_op) = children
        .iter()
        .find(|child| child.as_rule() == Rule::string_match_op)
    {
        return build_string_match(operand, string_op, children, source_span);
    }

    if children.iter().any(|child| child.as_rule() == Rule::in_kw) {
        if let Some(list_pair) = children
            .iter()
            .find(|child| child.as_rule() == Rule::list_lit)
            .cloned()
        {
            return Ok(ValueExpr::InList {
                operand: Box::new(operand),
                list: literal::build_list_items(list_pair)?,
                negated,
                span: source_span,
            });
        }
        let list_expr = find_child(
            children,
            Rule::comparison,
            "IN predicate is missing list expression",
        )?;
        return Ok(ValueExpr::InListExpression {
            operand: Box::new(operand),
            list: Box::new(build_value_expr(list_expr)?),
            negated,
            span: source_span,
        });
    }

    Ok(ValueExpr::IsCheck {
        operand: Box::new(operand),
        kind: build_is_kind(children, source_span)?,
        negated,
        span: source_span,
    })
}

fn build_string_match(
    operand: ValueExpr,
    string_op: &Pair<'_, Rule>,
    children: &[Pair<'_, Rule>],
    source_span: SourceSpan,
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
        rhs: Box::new(build_value_expr(rhs_pair)?),
        span: source_span,
    })
}

fn build_is_kind(
    children: &[Pair<'_, Rule>],
    source_span: SourceSpan,
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
        return Ok(IsCheckKind::Labeled(pattern::build_label_expr(label_pair)?));
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
        return Ok(IsCheckKind::SourceOf(Box::new(build_value_expr(rhs_pair)?)));
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
            rhs_pair,
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
