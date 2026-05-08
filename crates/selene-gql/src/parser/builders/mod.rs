//! Pair-to-AST builders.

pub(super) mod expr;
pub(super) mod pattern;

use pest::iterators::Pair;
use selene_core::{IStr, intern};

use crate::{
    ast::{
        LetBinding, LimitValue, NullsPolicy, OrderDirection, OrderTerm, PipelineStatement,
        QueryPipeline, ReturnClause, ReturnItem, SetOp, SourceSpan, Statement, UnwindStatement,
        WithClause,
    },
    error::ParserError,
};

use super::Rule;

pub(crate) fn build_statement(program_pair: Pair<'_, Rule>) -> Result<Statement, ParserError> {
    match program_pair.as_rule() {
        Rule::gql_program => {
            let child = first_non_eoi(program_pair)?;
            build_statement(child)
        }
        Rule::query_pipeline => build_query_pipeline(program_pair).map(Statement::Query),
        Rule::composite_query => build_composite(program_pair),
        Rule::chained_query => build_chained(program_pair),
        Rule::pipeline_statement => {
            let span = span(&program_pair);
            let statement = build_pipeline_statement(program_pair)?;
            Ok(Statement::Query(QueryPipeline {
                statements: vec![statement],
                span,
            }))
        }
        Rule::select_stmt => build_select_pipeline(program_pair).map(Statement::Query),
        Rule::mutation_pipeline => Err(not_implemented(
            &program_pair,
            "mutation statements land in BRIEF-18",
        )),
        Rule::ddl_statement => Err(not_implemented(&program_pair, "DDL lands in BRIEF-18")),
        Rule::transaction_control => Err(not_implemented(
            &program_pair,
            "transaction control lands in BRIEF-18",
        )),
        _ => Err(unexpected_pair(program_pair, "expected a GQL program")),
    }
}

fn build_composite(pair: Pair<'_, Rule>) -> Result<Statement, ParserError> {
    let source_span = span(&pair);
    let mut children = pair.into_inner();
    let first = children
        .next()
        .ok_or_else(ParserError::empty_program)
        .and_then(build_query_pipeline)?;
    let mut rest = Vec::new();

    while let Some(op_pair) = children.next() {
        let op = build_set_op(op_pair)?;
        let pipeline = children
            .next()
            .ok_or_else(ParserError::empty_program)
            .and_then(build_query_pipeline)?;
        rest.push((op, pipeline));
    }

    Ok(Statement::Composite {
        first,
        rest,
        span: source_span,
    })
}

fn build_chained(pair: Pair<'_, Rule>) -> Result<Statement, ParserError> {
    let source_span = span(&pair);
    let blocks = pair
        .into_inner()
        .filter(|child| child.as_rule() == Rule::query_pipeline)
        .map(build_query_pipeline)
        .collect::<Result<Vec<_>, _>>()?;
    if blocks.is_empty() {
        return Err(ParserError::empty_program());
    }
    Ok(Statement::Chained {
        blocks,
        span: source_span,
    })
}

fn build_set_op(pair: Pair<'_, Rule>) -> Result<SetOp, ParserError> {
    let child = pair
        .into_inner()
        .next()
        .ok_or_else(|| ParserError::syntax("set operator is empty", SourceSpan::default(), None))?;
    let has_all = contains_word(child.as_str(), "ALL");
    match child.as_rule() {
        Rule::union_op if has_all => Ok(SetOp::UnionAll),
        Rule::union_op => Ok(SetOp::Union),
        Rule::intersect_op if has_all => Ok(SetOp::IntersectAll),
        Rule::intersect_op => Ok(SetOp::Intersect),
        Rule::except_op if has_all => Ok(SetOp::ExceptAll),
        Rule::except_op => Ok(SetOp::Except),
        Rule::otherwise_op => Ok(SetOp::Otherwise),
        _ => Err(unexpected_pair(child, "expected set operator")),
    }
}

/// Case-insensitive whole-word match against a SQL/GQL keyword.
///
/// Used to detect modifier keywords (e.g. `ALL` in `INTERSECT ALL`) without
/// false-matching identifiers that happen to contain the keyword as a
/// substring.
fn contains_word(text: &str, word: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    upper
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .any(|part| part == word)
}

pub(super) fn build_query_pipeline(pair: Pair<'_, Rule>) -> Result<QueryPipeline, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::query_pipeline);
    let source_span = span(&pair);
    let statements = pair
        .into_inner()
        .map(build_pipeline_statement)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(QueryPipeline {
        statements,
        span: source_span,
    })
}

fn build_pipeline_statement(pair: Pair<'_, Rule>) -> Result<PipelineStatement, ParserError> {
    if pair.as_rule() == Rule::pipeline_statement {
        return build_pipeline_statement(first_child(pair)?);
    }

    match pair.as_rule() {
        Rule::match_stmt => pattern::build_match_clause(pair).map(PipelineStatement::Match),
        Rule::filter_stmt => build_filter(pair).map(PipelineStatement::Filter),
        Rule::let_stmt => build_let(pair).map(PipelineStatement::Let),
        Rule::unwind_stmt => build_unwind(pair).map(PipelineStatement::Unwind),
        Rule::sorting_stmt => build_sorting(pair).map(PipelineStatement::Sorting),
        Rule::offset_stmt => build_limit_or_offset(pair).map(PipelineStatement::Offset),
        Rule::limit_stmt => build_limit_or_offset(pair).map(PipelineStatement::Limit),
        Rule::return_stmt => build_return_clause(pair).map(PipelineStatement::Return),
        Rule::with_stmt => build_with_clause(pair).map(PipelineStatement::With),
        Rule::for_stmt => Err(not_implemented(&pair, "FOR lands in BRIEF-18")),
        Rule::match_view_stmt => Err(not_implemented(&pair, "MATCH VIEW lands in BRIEF-18")),
        Rule::call_stmt => Err(not_implemented(&pair, "CALL lands in BRIEF-18")),
        _ => Err(unexpected_pair(pair, "expected pipeline statement")),
    }
}

fn build_select_pipeline(pair: Pair<'_, Rule>) -> Result<QueryPipeline, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::select_stmt);
    let source_span = span(&pair);
    let mut return_clause = ReturnClause {
        distinct: false,
        star: false,
        items: Vec::new(),
        group_by: None,
        having: None,
        span: source_span,
    };
    // SELECT desugars to a query pipeline whose statements are emitted in
    // semantic order: pre-projection (MATCH, WHERE/FILTER), then RETURN, then
    // post-projection (ORDER BY, OFFSET, LIMIT). Routing WHERE through the
    // post-RETURN slot would filter on projected rows instead of source rows
    // and silently change query semantics whenever projection introduces
    // aliases or aggregation.
    let mut pre_return = Vec::new();
    let mut post_return = Vec::new();

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::distinct_kw => return_clause.distinct = true,
            Rule::return_star => return_clause.star = true,
            Rule::projection_list => return_clause.items = build_projection_list(child)?,
            Rule::select_from => {
                let from_child = first_child(child)?;
                if from_child.as_rule() == Rule::match_stmt {
                    pre_return.push(PipelineStatement::Match(pattern::build_match_clause(
                        from_child,
                    )?));
                } else {
                    return Err(not_implemented(
                        &from_child,
                        "SELECT FROM graph-name resolution lands in M5b",
                    ));
                }
            }
            Rule::where_clause => pre_return.push(PipelineStatement::Filter(build_where(child)?)),
            Rule::group_by_clause => return_clause.group_by = Some(build_group_by(child)?),
            Rule::having_clause => return_clause.having = Some(build_having(child)?),
            Rule::sorting_stmt => {
                post_return.push(PipelineStatement::Sorting(build_sorting(child)?));
            }
            Rule::offset_stmt => {
                post_return.push(PipelineStatement::Offset(build_limit_or_offset(child)?));
            }
            Rule::limit_stmt => {
                post_return.push(PipelineStatement::Limit(build_limit_or_offset(child)?));
            }
            _ => return Err(unexpected_pair(child, "unexpected SELECT child")),
        }
    }

    let mut statements = pre_return;
    statements.push(PipelineStatement::Return(return_clause));
    statements.extend(post_return);
    Ok(QueryPipeline {
        statements,
        span: source_span,
    })
}

fn build_filter(pair: Pair<'_, Rule>) -> Result<crate::ast::ValueExpr, ParserError> {
    expr_from_first(pair, "FILTER is missing expression")
}

fn build_where(pair: Pair<'_, Rule>) -> Result<crate::ast::ValueExpr, ParserError> {
    expr_from_first(pair, "WHERE is missing expression")
}

fn build_having(pair: Pair<'_, Rule>) -> Result<crate::ast::ValueExpr, ParserError> {
    expr_from_first(pair, "HAVING is missing expression")
}

fn expr_from_first(
    pair: Pair<'_, Rule>,
    missing: &'static str,
) -> Result<crate::ast::ValueExpr, ParserError> {
    let pair_span = span(&pair);
    let expr_pair = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::expr)
        .ok_or_else(|| ParserError::syntax(missing, pair_span, None))?;
    expr::build_value_expr(expr_pair)
}

fn build_let(pair: Pair<'_, Rule>) -> Result<Vec<LetBinding>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::let_binding)
        .map(|binding| {
            let binding_span = span(&binding);
            let mut children = binding.into_inner();
            let alias_pair = children.next().ok_or_else(|| {
                ParserError::syntax("LET binding is missing alias", binding_span, None)
            })?;
            let value_pair = children.next().ok_or_else(|| {
                ParserError::syntax("LET binding is missing expression", binding_span, None)
            })?;
            Ok(LetBinding {
                alias: intern_pair(alias_pair)?,
                value: expr::build_value_expr(value_pair)?,
                span: binding_span,
            })
        })
        .collect()
}

fn build_unwind(pair: Pair<'_, Rule>) -> Result<UnwindStatement, ParserError> {
    let source_span = span(&pair);
    let mut children = pair.into_inner();
    let source = children
        .next()
        .ok_or_else(|| {
            ParserError::syntax("UNWIND is missing source expression", source_span, None)
        })
        .and_then(expr::build_value_expr)?;
    let alias = children
        .next()
        .ok_or_else(|| ParserError::syntax("UNWIND is missing alias", source_span, None))
        .and_then(intern_pair)?;
    Ok(UnwindStatement {
        source,
        alias,
        span: source_span,
    })
}

fn build_sorting(pair: Pair<'_, Rule>) -> Result<Vec<OrderTerm>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::order_term)
        .map(build_order_term)
        .collect()
}

fn build_order_term(pair: Pair<'_, Rule>) -> Result<OrderTerm, ParserError> {
    let source_span = span(&pair);
    let mut expr_value = None;
    let mut direction = OrderDirection::Asc;
    let mut nulls = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::expr => expr_value = Some(expr::build_value_expr(child)?),
            Rule::sort_dir if child.as_str().eq_ignore_ascii_case("DESC") => {
                direction = OrderDirection::Desc;
            }
            Rule::sort_dir => direction = OrderDirection::Asc,
            Rule::nulls_order if child.as_str().to_ascii_uppercase().contains("FIRST") => {
                nulls = Some(NullsPolicy::NullsFirst);
            }
            Rule::nulls_order => nulls = Some(NullsPolicy::NullsLast),
            _ => return Err(unexpected_pair(child, "unexpected ORDER BY child")),
        }
    }

    Ok(OrderTerm {
        expr: expr_value.ok_or_else(|| {
            ParserError::syntax("ORDER BY term is missing expression", source_span, None)
        })?,
        direction,
        nulls,
        span: source_span,
    })
}

fn build_limit_or_offset(pair: Pair<'_, Rule>) -> Result<LimitValue, ParserError> {
    let source_span = span(&pair);
    let child = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::limit_value)
        .ok_or_else(|| ParserError::syntax("LIMIT/OFFSET is missing value", source_span, None))?;
    let inner = first_child(child)?;
    match inner.as_rule() {
        Rule::uint => inner
            .as_str()
            .parse::<u64>()
            .map(|value| LimitValue::Count(value, span(&inner)))
            .map_err(|error| {
                ParserError::syntax(format!("invalid LIMIT/OFFSET: {error}"), span(&inner), None)
            }),
        Rule::param_ref => {
            let value_span = span(&inner);
            Ok(LimitValue::Parameter(intern_param(inner)?, value_span))
        }
        _ => Err(unexpected_pair(inner, "expected LIMIT/OFFSET value")),
    }
}

fn build_return_clause(pair: Pair<'_, Rule>) -> Result<ReturnClause, ParserError> {
    let source_span = span(&pair);
    let mut clause = ReturnClause {
        distinct: false,
        star: false,
        items: Vec::new(),
        group_by: None,
        having: None,
        span: source_span,
    };

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::distinct_kw => clause.distinct = true,
            Rule::return_star => clause.star = true,
            Rule::projection_list => clause.items = build_projection_list(child)?,
            Rule::group_by_clause => clause.group_by = Some(build_group_by(child)?),
            Rule::having_clause => clause.having = Some(build_having(child)?),
            Rule::no_bindings => {
                return Err(not_implemented(
                    &child,
                    "RETURN NO BINDINGS lands in BRIEF-18",
                ));
            }
            _ => return Err(unexpected_pair(child, "unexpected RETURN child")),
        }
    }

    if !clause.star && clause.items.is_empty() {
        return Err(ParserError::syntax(
            "RETURN requires at least one projection item",
            source_span,
            None,
        ));
    }
    Ok(clause)
}

fn build_with_clause(pair: Pair<'_, Rule>) -> Result<WithClause, ParserError> {
    let source_span = span(&pair);
    let mut clause = WithClause {
        distinct: false,
        items: Vec::new(),
        group_by: None,
        having: None,
        where_clause: None,
        span: source_span,
    };

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::distinct_kw => clause.distinct = true,
            Rule::projection_list => clause.items = build_projection_list(child)?,
            Rule::group_by_clause => clause.group_by = Some(build_group_by(child)?),
            Rule::having_clause => clause.having = Some(build_having(child)?),
            Rule::where_clause => clause.where_clause = Some(build_where(child)?),
            _ => return Err(unexpected_pair(child, "unexpected WITH child")),
        }
    }
    Ok(clause)
}

fn build_projection_list(pair: Pair<'_, Rule>) -> Result<Vec<ReturnItem>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::projection)
        .map(build_return_item)
        .collect()
}

fn build_return_item(pair: Pair<'_, Rule>) -> Result<ReturnItem, ParserError> {
    let source_span = span(&pair);
    let mut value = None;
    let mut alias = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::expr => value = Some(expr::build_value_expr(child)?),
            Rule::alias => alias = Some(build_alias(child)?),
            _ => {
                return Err(unexpected_pair(
                    child,
                    "expected RETURN expression or alias",
                ));
            }
        }
    }

    Ok(ReturnItem {
        expr: value.ok_or_else(|| {
            ParserError::syntax("RETURN item is missing expression", source_span, None)
        })?,
        alias,
        span: source_span,
    })
}

fn build_alias(pair: Pair<'_, Rule>) -> Result<IStr, ParserError> {
    let source_span = span(&pair);
    let ident = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::prop_ident)
        .ok_or_else(|| ParserError::syntax("AS alias is missing identifier", source_span, None))?;
    intern_pair(ident)
}

fn build_group_by(pair: Pair<'_, Rule>) -> Result<Vec<crate::ast::ValueExpr>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::group_by_item)
        .map(|item| expr_from_first(item, "GROUP BY item is missing expression"))
        .collect()
}

pub(super) fn first_child(pair: Pair<'_, Rule>) -> Result<Pair<'_, Rule>, ParserError> {
    let pair_span = span(&pair);
    pair.into_inner()
        .next()
        .ok_or_else(|| ParserError::syntax("expected child rule", pair_span, None))
}

fn first_non_eoi(pair: Pair<'_, Rule>) -> Result<Pair<'_, Rule>, ParserError> {
    let pair_span = span(&pair);
    pair.into_inner()
        .find(|child| child.as_rule() != Rule::EOI)
        .ok_or_else(|| ParserError::syntax("empty GQL program", pair_span, None))
}

pub(super) fn span(pair: &Pair<'_, Rule>) -> SourceSpan {
    SourceSpan::from_pest(pair.as_span())
}

pub(super) fn intern_pair(pair: Pair<'_, Rule>) -> Result<IStr, ParserError> {
    let source_span = span(&pair);
    let decoded = decode_ident_like(pair.as_str());
    intern(&decoded).map_err(|error| {
        ParserError::syntax(
            format!("could not intern identifier: {error}"),
            source_span,
            Some("identifier interning cap may be exhausted".into()),
        )
    })
}

pub(super) fn intern_param(pair: Pair<'_, Rule>) -> Result<IStr, ParserError> {
    let source_span = span(&pair);
    let text = pair.as_str().strip_prefix('$').unwrap_or(pair.as_str());
    intern(text).map_err(|error| {
        ParserError::syntax(
            format!("could not intern parameter: {error}"),
            source_span,
            Some("identifier interning cap may be exhausted".into()),
        )
    })
}

pub(super) fn decode_ident_like(text: &str) -> String {
    if let Some(inner) = text.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        inner.replace("\"\"", "\"")
    } else if let Some(inner) = text.strip_prefix('`').and_then(|s| s.strip_suffix('`')) {
        inner.to_owned()
    } else {
        text.to_owned()
    }
}

pub(super) fn unexpected_pair(pair: Pair<'_, Rule>, message: &'static str) -> ParserError {
    ParserError::syntax(message, span(&pair), None)
}

pub(super) fn not_implemented(pair: &Pair<'_, Rule>, message: &'static str) -> ParserError {
    ParserError::not_implemented(
        message,
        span(pair),
        Some("see BRIEF-17 / BRIEF-18 scope split"),
    )
}
