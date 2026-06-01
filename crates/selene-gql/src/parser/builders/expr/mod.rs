//! Expression pair-to-AST builders.

mod call;
mod literal;
mod predicate;

use pest::iterators::Pair;

use crate::{
    ast::{BinaryOp, GqlType, Literal, SourceSpan, UnaryOp, ValueExpr},
    error::ParserError,
};

use super::{
    Rule, build_typed_param_ref, first_child, intern_pair, intern_param, not_implemented, span,
    unexpected_pair,
};

pub(super) fn build_value_expr(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    match pair.as_rule() {
        Rule::expr => build_value_expr(first_child(pair)?),
        Rule::or_expr => build_left_assoc(pair, Rule::or_kw, |_| BinaryOp::Or),
        Rule::xor_expr => build_left_assoc(pair, Rule::xor_kw, |_| BinaryOp::Xor),
        Rule::and_expr => build_left_assoc(pair, Rule::and_kw, |_| BinaryOp::And),
        Rule::not_expr => build_not_expr(pair),
        Rule::is_expr => build_is_expr(pair),
        Rule::comparison => build_left_assoc(pair, Rule::comp_op, build_comparison_op),
        Rule::concat => build_repeated_same_op(pair, BinaryOp::Concat),
        Rule::addition => build_left_assoc(pair, Rule::add_op, build_add_op),
        Rule::multiplication => build_left_assoc(pair, Rule::mul_op, build_mul_op),
        Rule::unary => build_unary(pair),
        Rule::postfix => build_postfix(pair),
        Rule::primary => build_value_expr(first_child(pair)?),
        Rule::literal => literal::build_literal_expr(pair),
        Rule::list_lit => literal::build_list_lit(pair),
        Rule::record_constructor => build_record_constructor(pair),
        Rule::var_ref => Ok(ValueExpr::Variable {
            name: intern_pair(pair)?,
            span: source_span,
        }),
        Rule::param_ref => Ok(ValueExpr::Parameter {
            name: intern_param(pair)?,
            declared_type: None,
            span: source_span,
        }),
        Rule::typed_param_ref => {
            let (name, declared_type, span) = build_typed_param_ref(pair)?;
            Ok(ValueExpr::Parameter {
                name,
                declared_type,
                span,
            })
        }
        Rule::function_call => call::build_function_call(pair),
        Rule::normalize_expr => call::build_normalize_expr(pair),
        Rule::aggregate_expr => call::build_aggregate_expr(first_child(pair)?),
        Rule::paren_expr => build_value_expr(first_child(pair)?),
        Rule::all_different_expr => {
            call::build_expr_list_predicate(pair, call::PredicateKind::AllDifferent)
        }
        Rule::same_expr => call::build_expr_list_predicate(pair, call::PredicateKind::Same),
        Rule::property_exists_expr => call::build_property_exists(pair),
        Rule::exists_expr => call::build_exists(pair),
        Rule::count_subquery_expr => call::build_count_subquery(pair),
        Rule::value_subquery_expr => call::build_value_subquery(pair),
        Rule::case_expr => call::build_case_expr(first_child(pair)?),
        Rule::simple_case | Rule::searched_case => call::build_case_expr(pair),
        Rule::cast_expr => build_cast_expr(pair),
        Rule::labels_expr => Err(not_implemented(
            &pair,
            "LABELS expressions are not yet supported",
        )),
        Rule::trim_expr => call::build_trim_expr(pair),
        Rule::list_iter_expr | Rule::list_comprehension | Rule::list_quant | Rule::list_reduce => {
            Err(not_implemented(
                &pair,
                "list-iteration expressions are not yet supported",
            ))
        }
        _ => Err(unexpected_pair(pair, "expected value expression")),
    }
}

pub(super) fn build_type_name(pair: Pair<'_, Rule>) -> Result<GqlType, ParserError> {
    predicate::build_type_name(pair)
}

fn build_cast_expr(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let mut value_pair: Option<Pair<'_, Rule>> = None;
    let mut type_pair: Option<Pair<'_, Rule>> = None;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::expr => value_pair = Some(child),
            Rule::type_name => type_pair = Some(child),
            _ => {}
        }
    }
    let value_pair = value_pair.ok_or_else(|| {
        ParserError::syntax("CAST is missing source expression", source_span, None)
    })?;
    let type_pair = type_pair
        .ok_or_else(|| ParserError::syntax("CAST is missing target type", source_span, None))?;
    let value = build_value_expr(value_pair)?;
    let target_type = build_type_name(type_pair)?;
    Ok(ValueExpr::Cast {
        value: Box::new(value),
        target_type: Box::new(target_type),
        span: source_span,
    })
}

/// Decode a `string_lit` pair into raw text (unquoted, escapes resolved).
pub(super) fn decode_string_text(pair: &Pair<'_, Rule>) -> Result<String, ParserError> {
    literal::decode_string_text(pair)
}

fn build_left_assoc(
    pair: Pair<'_, Rule>,
    op_rule: Rule,
    op_from_pair: fn(&Pair<'_, Rule>) -> BinaryOp,
) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let mut children = pair.into_inner();
    let first = children
        .next()
        .ok_or_else(|| ParserError::syntax("expression is empty", source_span, None))?;
    let mut value = build_value_expr(first)?;

    while let Some(op_pair) = children.next() {
        if op_pair.as_rule() != op_rule {
            return Err(unexpected_pair(op_pair, "expected binary operator"));
        }
        let rhs_pair = children.next().ok_or_else(|| {
            ParserError::syntax(
                "binary operator is missing right operand",
                source_span,
                None,
            )
        })?;
        let rhs = build_value_expr(rhs_pair)?;
        value = binary(value, op_from_pair(&op_pair), rhs);
    }

    Ok(value)
}

fn build_repeated_same_op(pair: Pair<'_, Rule>, op: BinaryOp) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let mut children = pair.into_inner();
    let first = children
        .next()
        .ok_or_else(|| ParserError::syntax("expression is empty", source_span, None))?;
    let mut value = build_value_expr(first)?;

    for rhs_pair in children {
        value = binary(value, op, build_value_expr(rhs_pair)?);
    }

    Ok(value)
}

fn binary(lhs: ValueExpr, op: BinaryOp, rhs: ValueExpr) -> ValueExpr {
    let span = SourceSpan::merge(lhs.span(), rhs.span());
    ValueExpr::BinaryOp {
        op,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
        span,
    }
}

fn build_comparison_op(pair: &Pair<'_, Rule>) -> BinaryOp {
    match pair.as_str() {
        "=" => BinaryOp::Eq,
        "<>" => BinaryOp::Ne,
        "<" => BinaryOp::Lt,
        "<=" => BinaryOp::Le,
        ">" => BinaryOp::Gt,
        ">=" => BinaryOp::Ge,
        _ => BinaryOp::Eq,
    }
}

fn build_add_op(pair: &Pair<'_, Rule>) -> BinaryOp {
    match pair.as_str() {
        "-" => BinaryOp::Sub,
        _ => BinaryOp::Add,
    }
}

fn build_mul_op(pair: &Pair<'_, Rule>) -> BinaryOp {
    match pair.as_str() {
        "/" => BinaryOp::Div,
        _ => BinaryOp::Mul,
    }
}

fn build_not_expr(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let mut children = pair.into_inner();
    let Some(first) = children.next() else {
        return Err(ParserError::syntax(
            "expression is empty",
            source_span,
            None,
        ));
    };

    if first.as_rule() == Rule::not_kw {
        let operand_pair = children.next().ok_or_else(|| {
            ParserError::syntax("NOT operator is missing operand", source_span, None)
        })?;
        return Ok(ValueExpr::UnaryOp {
            op: UnaryOp::Not,
            operand: Box::new(build_value_expr(operand_pair)?),
            span: source_span,
        });
    }

    build_value_expr(first)
}

fn build_is_expr(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let mut children = pair.into_inner();
    let operand_pair = children
        .next()
        .ok_or_else(|| ParserError::syntax("IS expression is empty", source_span, None))?;
    let operand = build_value_expr(operand_pair)?;
    match children.next() {
        Some(suffix) => predicate::apply_is_suffix(operand, suffix, source_span),
        None => Ok(operand),
    }
}

fn build_unary(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let mut children = pair.into_inner();
    let first = children
        .next()
        .ok_or_else(|| ParserError::syntax("expression is empty", source_span, None))?;

    if first.as_rule() != Rule::sign_op {
        return build_value_expr(first);
    }

    let is_negative = first.as_str() == "-";
    let operand_pair = children.next().ok_or_else(|| {
        ParserError::syntax("unary operator is missing operand", source_span, None)
    })?;
    let operand = build_value_expr(operand_pair)?;

    match (is_negative, operand) {
        (false, value @ ValueExpr::Literal(Literal::Integer(_, _)))
        | (false, value @ ValueExpr::Literal(Literal::Float(_, _))) => {
            Ok(literal::with_numeric_span(value, source_span))
        }
        (true, ValueExpr::Literal(Literal::Integer(value, _))) => {
            let signed = value.checked_neg().ok_or_else(|| {
                ParserError::syntax(
                    "integer literal overflows i64 after negation",
                    source_span,
                    Some("integer literals must fit in i64".into()),
                )
            })?;
            Ok(ValueExpr::Literal(Literal::Integer(signed, source_span)))
        }
        (true, ValueExpr::Literal(Literal::Float(value, _))) => {
            Ok(ValueExpr::Literal(Literal::Float(-value, source_span)))
        }
        (false, value) => Ok(value),
        (true, value) => Ok(ValueExpr::UnaryOp {
            op: UnaryOp::Negate,
            operand: Box::new(value),
            span: source_span,
        }),
    }
}

fn build_postfix(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let mut children = pair.into_inner();
    let first = children
        .next()
        .ok_or_else(|| ParserError::syntax("postfix expression is empty", source_span, None))?;
    let mut value = build_value_expr(first)?;

    for op_pair in children {
        let op_span = span(&op_pair);
        let op_child = first_child(op_pair)?;
        match op_child.as_rule() {
            Rule::prop_access => {
                let prop = first_child(op_child)?;
                let previous_span = value.span();
                value = ValueExpr::PropertyAccess {
                    target: Box::new(value),
                    key: intern_pair(prop)?,
                    span: SourceSpan::merge(previous_span, op_span),
                };
            }
            Rule::list_access_op => {
                let index_pair = first_child(op_child)?;
                let previous_span = value.span();
                value = ValueExpr::ListAccess {
                    target: Box::new(value),
                    index: Box::new(build_value_expr(index_pair)?),
                    span: SourceSpan::merge(previous_span, op_span),
                };
            }
            _ => return Err(unexpected_pair(op_child, "expected postfix operator")),
        }
    }

    Ok(value)
}

fn build_record_constructor(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let fields = pair
        .into_inner()
        .filter(|child| child.as_rule() == Rule::record_field)
        .map(|field| {
            let field_span = span(&field);
            let mut children = field.into_inner();
            let key_pair = children.next().ok_or_else(|| {
                ParserError::syntax("record field is missing name", field_span, None)
            })?;
            let value_pair = children.next().ok_or_else(|| {
                ParserError::syntax("record field is missing value", field_span, None)
            })?;
            Ok((intern_pair(key_pair)?, build_value_expr(value_pair)?))
        })
        .collect::<Result<Vec<_>, ParserError>>()?;
    Ok(ValueExpr::RecordLiteral {
        fields,
        span: source_span,
    })
}
