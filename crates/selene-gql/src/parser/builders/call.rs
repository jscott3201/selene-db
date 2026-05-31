//! Procedure-call builders.

use pest::iterators::Pair;

use selene_core::IStr;

use crate::{
    ast::{
        InlineProcedureCall, PipelineStatement, ProcedureCall, QueryPipeline, Statement,
        YieldColumn, YieldItem, util::NonEmpty,
    },
    error::ParserError,
    parser::budget::InternerBudget,
};

use super::{
    Rule, build_qualified_name, build_query_pipeline, expr, first_child, intern_pair,
    not_implemented, span, unexpected_pair,
};

pub(super) fn build_top_level_call(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<Statement, ParserError> {
    let source_span = span(&pair);
    match build_call_stmt(pair, budget)? {
        BuiltCall::Procedure(call) => Ok(Statement::Call(call)),
        BuiltCall::Inline(call) => Ok(Statement::Query(QueryPipeline {
            statements: vec![PipelineStatement::CallSubquery(call)],
            span: source_span,
        })),
    }
}

pub(super) fn build_pipeline_call(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<PipelineStatement, ParserError> {
    match build_call_stmt(pair, budget)? {
        BuiltCall::Procedure(call) => Ok(PipelineStatement::Call(call)),
        BuiltCall::Inline(call) => Ok(PipelineStatement::CallSubquery(call)),
    }
}

enum BuiltCall {
    Procedure(ProcedureCall),
    Inline(InlineProcedureCall),
}

fn build_call_stmt(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<BuiltCall, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::call_stmt);
    let inner = first_child(pair)?;
    match inner.as_rule() {
        Rule::call_procedure => build_procedure_call(inner, budget).map(BuiltCall::Procedure),
        Rule::call_subquery => build_inline_call(inner, budget).map(BuiltCall::Inline),
        _ => Err(unexpected_pair(inner, "expected CALL body")),
    }
}

fn build_inline_call(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<InlineProcedureCall, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::call_subquery);
    let source_span = span(&pair);
    let mut variable_scope = None;
    let mut body = None;
    let mut yield_items = Vec::new();
    let mut in_transactions = false;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::variable_scope_clause => {
                variable_scope = Some(build_variable_scope(child, budget)?);
            }
            Rule::query_pipeline => body = Some(build_query_pipeline(child, budget)?),
            Rule::yield_clause => yield_items = build_yield_items(child, budget)?,
            Rule::call_transactions_clause => in_transactions = true,
            _ => return Err(unexpected_pair(child, "unexpected CALL subquery child")),
        }
    }

    Ok(InlineProcedureCall {
        variable_scope,
        body: Box::new(body.ok_or_else(|| {
            ParserError::syntax("CALL subquery is missing body", source_span, None)
        })?),
        yield_items,
        in_transactions,
        span: source_span,
    })
}

fn build_variable_scope(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<Vec<IStr>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::ident)
        .map(|child| intern_pair(child, budget))
        .collect()
}

fn build_procedure_call(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<ProcedureCall, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::call_procedure);
    let source_span = span(&pair);
    let mut name = None;
    let mut args = Vec::new();
    let mut yield_items = Vec::new();

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::qualified_name => name = Some(build_qualified_name(child, budget)?),
            Rule::arg_list => {
                args = child
                    .into_inner()
                    .filter(|arg| arg.as_rule() == Rule::expr)
                    .map(|arg| expr::build_value_expr(arg, budget))
                    .collect::<Result<Vec<_>, _>>()?;
            }
            Rule::yield_clause => yield_items = build_yield_items(child, budget)?,
            Rule::yield_filter => {
                return Err(not_implemented(
                    &child,
                    "YIELD WHERE filters are not yet supported",
                ));
            }
            _ => return Err(unexpected_pair(child, "unexpected procedure-call child")),
        }
    }

    Ok(ProcedureCall {
        name: NonEmpty::try_from_vec(name.ok_or_else(|| {
            ParserError::syntax("procedure call is missing name", source_span, None)
        })?)
        .expect("grammar guarantees >= 1: qualified_name"),
        args,
        yield_items,
        span: source_span,
    })
}

pub(super) fn build_yield_items(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<Vec<YieldItem>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::yield_item)
        .map(|child| build_yield_item(child, budget))
        .collect()
}

fn build_yield_item(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<YieldItem, ParserError> {
    let source_span = span(&pair);
    let is_star = pair.as_str().trim_start().starts_with('*');
    let mut column = if is_star {
        Some(YieldColumn::Star)
    } else {
        None
    };
    let mut alias = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::prop_ident if column.is_none() => {
                column = Some(YieldColumn::Named(intern_pair(child, budget)?));
            }
            Rule::alias => alias = Some(intern_pair(first_child(child)?, budget)?),
            _ => return Err(unexpected_pair(child, "unexpected YIELD item child")),
        }
    }

    Ok(YieldItem {
        column: column.ok_or_else(|| {
            ParserError::syntax("YIELD item is missing column", source_span, None)
        })?,
        alias,
        span: source_span,
    })
}
