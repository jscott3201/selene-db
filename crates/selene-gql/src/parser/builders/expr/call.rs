//! Function, aggregate, subquery, and CASE builders.

use pest::iterators::Pair;
use selene_core::DbString;

use crate::{
    ast::{BinaryOp, NormalForm, SourceSpan, TrimSpec, ValueExpr, util::NonEmpty},
    error::ParserError,
};

use super::{Rule, build_value_expr, first_child, literal};
use crate::parser::builders::{
    build_qualified_name, build_query_pipeline, db_string_from_owned, pattern, span,
    unexpected_pair,
};

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
    let name = build_qualified_name(name_pair)?;
    let mut args = Vec::new();
    for child in children {
        match child.as_rule() {
            Rule::arg_list => {
                args = child
                    .into_inner()
                    .filter(|arg| arg.as_rule() == Rule::expr)
                    .map(|arg| build_value_expr(arg))
                    .collect::<Result<Vec<_>, _>>()?;
            }
            _ => return Err(unexpected_pair(child, "unexpected function-call child")),
        }
    }
    Ok(ValueExpr::FunctionCall {
        name: NonEmpty::try_from_vec(name).expect("grammar guarantees >= 1: qualified_name"),
        args,
        star: false,
        distinct: false,
        span: source_span,
    })
}

pub(super) fn build_current_datetime_function(
    pair: Pair<'_, Rule>,
) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let function_pair = first_child(pair)?;
    let name = match function_pair.as_rule() {
        Rule::current_date_function => "current_date",
        Rule::current_time_function => "current_time",
        Rule::current_timestamp_function => "current_timestamp",
        Rule::local_timestamp_function => "localtimestamp",
        Rule::local_time_function => "localtime",
        _ => {
            return Err(unexpected_pair(
                function_pair,
                "unexpected current-datetime function",
            ));
        }
    };
    Ok(ValueExpr::FunctionCall {
        name: NonEmpty::try_from_vec(vec![db_string_from_owned(
            name.to_owned(),
            source_span,
            "current-datetime function name",
        )?])
        .expect("literal vector is non-empty"),
        args: Vec::new(),
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
            Rule::aggregate_op | Rule::binary_aggregate_op => {
                name = Some(lowercase_db_string(child)?)
            }
            Rule::distinct_kw => distinct = true,
            Rule::star => star = true,
            Rule::expr => args.push(build_value_expr(child)?),
            _ => return Err(unexpected_pair(child, "unexpected aggregate child")),
        }
    }

    let segment = name.ok_or_else(|| {
        ParserError::syntax("aggregate expression is missing name", source_span, None)
    })?;
    validate_aggregate_shape(&segment, distinct, star, args.len(), source_span)?;
    Ok(ValueExpr::FunctionCall {
        name: NonEmpty::try_from_vec(vec![segment]).expect("grammar guarantees >= 1: aggregate_op"),
        args,
        star,
        distinct,
        span: source_span,
    })
}

fn validate_aggregate_shape(
    name: &DbString,
    distinct: bool,
    star: bool,
    arg_count: usize,
    span: SourceSpan,
) -> Result<(), ParserError> {
    if star {
        if name.as_str() == "count" && !distinct {
            return Ok(());
        }
        return Err(ParserError::syntax(
            "only COUNT(*) may use aggregate asterisk syntax",
            span,
            None,
        ));
    }
    if arg_count == 0 {
        return Err(ParserError::syntax(
            "aggregate function is missing value expression",
            span,
            None,
        ));
    }
    Ok(())
}

pub(super) fn build_normalize_expr(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let mut source = None;
    let mut form = None;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::expr => source = Some(build_value_expr(child)?),
            Rule::normal_form => form = Some(parse_normal_form(child.as_str())),
            _ => return Err(unexpected_pair(child, "unexpected NORMALIZE child")),
        }
    }
    Ok(ValueExpr::Normalize {
        source: Box::new(source.ok_or_else(|| {
            ParserError::syntax("NORMALIZE is missing source expression", source_span, None)
        })?),
        form,
        span: source_span,
    })
}

pub(super) fn build_trim_expr(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let mut spec = TrimSpec::Both;
    let mut values = Vec::new();
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::trim_spec => spec = parse_trim_spec(child.as_str()),
            Rule::expr => values.push(build_value_expr(child)?),
            Rule::trim_char => values.push(build_value_expr(first_child(child)?)?),
            _ => return Err(unexpected_pair(child, "unexpected TRIM child")),
        }
    }
    let source = values.pop().ok_or_else(|| {
        ParserError::syntax("TRIM is missing source expression", source_span, None)
    })?;
    let character = values.pop().map(Box::new);
    if !values.is_empty() {
        return Err(ParserError::syntax(
            "TRIM has too many value expressions",
            source_span,
            None,
        ));
    }
    Ok(ValueExpr::Trim {
        spec,
        character,
        source: Box::new(source),
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
        .map(|child| build_value_expr(child))
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

pub(super) fn build_value_subquery(pair: Pair<'_, Rule>) -> Result<ValueExpr, ParserError> {
    let source_span = span(&pair);
    let body_pair = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::query_pipeline)
        .ok_or_else(|| {
            ParserError::syntax("VALUE subquery is missing query body", source_span, None)
        })?;
    Ok(ValueExpr::ValueSubquery {
        body: Box::new(build_query_pipeline(body_pair)?),
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
        .and_then(|pair| build_value_expr(pair))
}

fn lowercase_db_string(pair: Pair<'_, Rule>) -> Result<DbString, ParserError> {
    let source_span = span(&pair);
    let canonical = pair.as_str().to_ascii_lowercase();
    db_string_from_owned(canonical, source_span, "aggregate name")
}

fn parse_normal_form(value: &str) -> NormalForm {
    match value.to_ascii_uppercase().as_str() {
        "NFD" => NormalForm::Nfd,
        "NFKC" => NormalForm::Nfkc,
        "NFKD" => NormalForm::Nfkd,
        _ => NormalForm::Nfc,
    }
}

fn parse_trim_spec(value: &str) -> TrimSpec {
    match value.to_ascii_uppercase().as_str() {
        "LEADING" => TrimSpec::Leading,
        "TRAILING" => TrimSpec::Trailing,
        _ => TrimSpec::Both,
    }
}
