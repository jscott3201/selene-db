//! Function, aggregate, subquery, and CASE builders.

use pest::iterators::Pair;
use selene_core::{IStr, intern};

use crate::{
    ast::{BinaryOp, SourceSpan, ValueExpr},
    error::ParserError,
};

use super::{Rule, build_value_expr, literal};
use crate::parser::builders::{decode_ident_like, pattern, span, unexpected_pair};

pub(super) enum PredicateKind {
    AllDifferent,
    Same,
}

pub(super) fn build_function_call(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let mut children = pair.into_inner();
    let name_pair = children
        .next()
        .ok_or_else(|| ParserError::syntax("function call is missing name", source_span, None))?;
    let name = intern_qualified_name(name_pair)?;
    let mut args = Vec::new();
    for child in children {
        match child.as_rule() {
            Rule::arg_list => {
                args = child
                    .into_inner()
                    .filter(|arg| arg.as_rule() == Rule::expr)
                    .map(build_value_expr)
                    .collect::<Result<Vec<_>, _>>()?;
            }
            _ => return Err(unexpected_pair(child, "unexpected function-call child")),
        }
    }
    Ok(ValueExpr::FunctionCall {
        name,
        args,
        star: false,
        distinct: false,
        span: source_span,
    })
}

pub(super) fn build_aggregate_expr(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let mut name = None;
    let mut distinct = false;
    let mut star = false;
    let mut args = Vec::new();

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::aggregate_op => name = Some(intern_lower(child)?),
            Rule::distinct_kw => distinct = true,
            Rule::star => star = true,
            Rule::expr => args.push(build_value_expr(child)?),
            _ => return Err(unexpected_pair(child, "unexpected aggregate child")),
        }
    }

    Ok(ValueExpr::FunctionCall {
        name: name.ok_or_else(|| {
            ParserError::syntax("aggregate expression is missing name", source_span, None)
        })?,
        args,
        star,
        distinct,
        span: source_span,
    })
}

pub(super) fn build_expr_list_predicate(
    pair: Pair<'_, Rule>,
    kind: PredicateKind,
) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let items = pair
        .into_inner()
        .filter(|child| child.as_rule() == Rule::expr)
        .map(build_value_expr)
        .collect::<Result<Vec<_>, _>>()?;
    match kind {
        PredicateKind::AllDifferent => Ok(ValueExpr::AllDifferent {
            items,
            span: source_span,
        }),
        PredicateKind::Same => Ok(ValueExpr::Same {
            items,
            span: source_span,
        }),
    }
}

pub(super) fn build_property_exists(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let mut target = None;
    let mut key = None;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::expr => target = Some(build_value_expr(child)?),
            Rule::string_lit => key = Some(literal::parse_string_pair(child)?),
            _ => return Err(unexpected_pair(child, "unexpected PROPERTY_EXISTS child")),
        }
    }
    Ok(ValueExpr::PropertyExists {
        target: Box::new(target.ok_or_else(|| {
            ParserError::syntax("PROPERTY_EXISTS is missing target", source_span, None)
        })?),
        key: key.ok_or_else(|| {
            ParserError::syntax("PROPERTY_EXISTS is missing property key", source_span, None)
        })?,
        span: source_span,
    })
}

pub(super) fn build_exists(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let negated = pair.as_str().to_ascii_uppercase().starts_with("NOT");
    let match_pair = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::match_stmt)
        .ok_or_else(|| ParserError::syntax("EXISTS is missing MATCH pattern", source_span, None))?;
    Ok(ValueExpr::Exists {
        pattern: Box::new(pattern::build_match_clause(match_pair)?),
        negated,
        span: source_span,
    })
}

pub(super) fn build_count_subquery(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let match_pair = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::match_stmt)
        .ok_or_else(|| {
            ParserError::syntax("COUNT subquery is missing MATCH pattern", source_span, None)
        })?;
    Ok(ValueExpr::CountSubquery {
        pattern: Box::new(pattern::build_match_clause(match_pair)?),
        span: source_span,
    })
}

pub(super) fn build_case_expr(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    match pair.as_rule() {
        Rule::simple_case => build_simple_case(pair, source_span),
        Rule::searched_case => build_searched_case(pair, source_span),
        _ => Err(unexpected_pair(pair, "expected CASE expression")),
    }
}

fn build_simple_case(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<ValueExpr, ParserError> {
    let mut children = pair.into_inner();
    let base =
        build_value_expr(children.next().ok_or_else(|| {
            ParserError::syntax("simple CASE is missing input", source_span, None)
        })?)?;
    let mut branches = Vec::new();
    let mut else_branch = None;

    for child in children {
        match child.as_rule() {
            Rule::simple_when => branches.push(simple_when_branch(child, &base)?),
            Rule::else_clause => else_branch = Some(Box::new(expr_from_child(child)?)),
            _ => return Err(unexpected_pair(child, "unexpected CASE child")),
        }
    }

    Ok(ValueExpr::Case {
        branches,
        else_branch,
        span: source_span,
    })
}

fn simple_when_branch(
    pair: Pair<'_, Rule>,
    base: &ValueExpr,
) -> Result<(ValueExpr, ValueExpr), ParserError> {
    let when_span = span(&pair);
    let mut children = pair.into_inner();
    let when_value =
        build_value_expr(children.next().ok_or_else(|| {
            ParserError::syntax("CASE WHEN is missing expression", when_span, None)
        })?)?;
    let then_value =
        build_value_expr(children.next().ok_or_else(|| {
            ParserError::syntax("CASE THEN is missing expression", when_span, None)
        })?)?;
    let condition_span = SourceSpan::merge(base.span(), when_value.span());
    Ok((
        ValueExpr::BinaryOp {
            op: BinaryOp::Eq,
            lhs: Box::new(base.clone()),
            rhs: Box::new(when_value),
            span: condition_span,
        },
        then_value,
    ))
}

fn build_searched_case(
    pair: Pair<'_, Rule>,
    source_span: SourceSpan,
) -> Result<ValueExpr, ParserError> {
    let mut branches = Vec::new();
    let mut else_branch = None;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::when_clause => branches.push(searched_when_branch(child)?),
            Rule::else_clause => else_branch = Some(Box::new(expr_from_child(child)?)),
            _ => return Err(unexpected_pair(child, "unexpected CASE child")),
        }
    }
    Ok(ValueExpr::Case {
        branches,
        else_branch,
        span: source_span,
    })
}

fn searched_when_branch(pair: Pair<'_, Rule>) -> Result<(ValueExpr, ValueExpr), ParserError> {
    let when_span = span(&pair);
    let mut children = pair.into_inner();
    let condition =
        build_value_expr(children.next().ok_or_else(|| {
            ParserError::syntax("CASE WHEN is missing condition", when_span, None)
        })?)?;
    let value = build_value_expr(
        children
            .next()
            .ok_or_else(|| ParserError::syntax("CASE THEN is missing value", when_span, None))?,
    )?;
    Ok((condition, value))
}

fn expr_from_child(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    pair.into_inner()
        .find(|child| child.as_rule() == Rule::expr)
        .ok_or_else(|| ParserError::syntax("clause is missing expression", source_span, None))
        .and_then(build_value_expr)
}

fn intern_qualified_name(pair: Pair<'_, Rule>) -> Result<IStr, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::qualified_name);
    let source_span = span(&pair);
    let mut parts = Vec::new();
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::ident | Rule::prop_ident => parts.push(decode_ident_like(child.as_str())),
            _ => return Err(unexpected_pair(child, "unexpected qualified-name child")),
        }
    }
    let joined = parts.join(".");
    intern(&joined).map_err(|error| {
        ParserError::syntax(
            format!("could not intern qualified name: {error}"),
            source_span,
            Some("identifier interning cap may be exhausted".into()),
        )
    })
}

fn intern_lower(pair: Pair<'_, Rule>) -> Result<IStr, ParserError> {
    let source_span = span(&pair);
    let canonical = pair.as_str().to_ascii_lowercase();
    intern(&canonical).map_err(|error| {
        ParserError::syntax(
            format!("could not intern aggregate name: {error}"),
            source_span,
            Some("identifier interning cap may be exhausted".into()),
        )
    })
}
