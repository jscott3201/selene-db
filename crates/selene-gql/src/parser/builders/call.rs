//! Procedure-call builders.

use pest::iterators::Pair;

use selene_core::DbString;

use crate::{
    ast::{
        InlineProcedureCall, PipelineStatement, ProcedureCall, QueryPipeline, Statement,
        YieldColumn, YieldItem, util::NonEmpty,
    },
    error::ParserError,
};

use super::{
    Rule, build_qualified_name, build_query_pipeline, db_string_pair, expr, first_child, span,
    unexpected_pair,
};

pub(super) fn build_top_level_call(pair: Pair<'_, Rule>) -> Result<Statement, ParserError> {
    let source_span = span(&pair);
    match build_call_stmt(pair)? {
        BuiltCall::Procedure(call) => Ok(Statement::Call(call)),
        BuiltCall::Inline(call) => Ok(Statement::Query(QueryPipeline {
            statements: vec![PipelineStatement::CallSubquery(call)],
            span: source_span,
        })),
    }
}

pub(super) fn build_pipeline_call(pair: Pair<'_, Rule>) -> Result<PipelineStatement, ParserError> {
    match build_call_stmt(pair)? {
        BuiltCall::Procedure(call) => Ok(PipelineStatement::Call(call)),
        BuiltCall::Inline(call) => Ok(PipelineStatement::CallSubquery(call)),
    }
}

enum BuiltCall {
    Procedure(ProcedureCall),
    Inline(InlineProcedureCall),
}

fn build_call_stmt(pair: Pair<'_, Rule>) -> Result<BuiltCall, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::call_stmt);
    let mut optional = false;
    let mut body = None;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::optional_modifier => optional = true,
            Rule::call_procedure | Rule::call_subquery => body = Some(child),
            _ => return Err(unexpected_pair(child, "expected CALL body")),
        }
    }
    let inner = body.ok_or_else(ParserError::empty_program)?;
    match inner.as_rule() {
        Rule::call_procedure => {
            let mut call = build_procedure_call(inner)?;
            call.optional = optional;
            Ok(BuiltCall::Procedure(call))
        }
        Rule::call_subquery => {
            let mut call = build_inline_call(inner)?;
            call.optional = optional;
            Ok(BuiltCall::Inline(call))
        }
        _ => Err(unexpected_pair(inner, "expected CALL body")),
    }
}

fn build_inline_call(pair: Pair<'_, Rule>) -> Result<InlineProcedureCall, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::call_subquery);
    let source_span = span(&pair);
    let mut variable_scope = None;
    let mut body = None;
    let mut yield_items = Vec::new();
    let mut in_transactions = false;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::variable_scope_clause => {
                variable_scope = Some(build_variable_scope(child)?);
            }
            Rule::query_pipeline => body = Some(build_query_pipeline(child)?),
            Rule::yield_clause => yield_items = build_yield_items(child)?,
            Rule::call_transactions_clause => in_transactions = true,
            _ => return Err(unexpected_pair(child, "unexpected CALL subquery child")),
        }
    }

    Ok(InlineProcedureCall {
        optional: false,
        variable_scope,
        body: Box::new(body.ok_or_else(|| {
            ParserError::syntax("CALL subquery is missing body", source_span, None)
        })?),
        yield_items,
        in_transactions,
        span: source_span,
    })
}

fn build_variable_scope(pair: Pair<'_, Rule>) -> Result<Vec<DbString>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::ident)
        .map(|child| db_string_pair(child))
        .collect()
}

fn build_procedure_call(pair: Pair<'_, Rule>) -> Result<ProcedureCall, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::call_procedure);
    let source_span = span(&pair);
    let mut name = None;
    let mut args = Vec::new();
    let mut yield_items = Vec::new();
    let mut yield_filter = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::qualified_name => name = Some(build_qualified_name(child)?),
            Rule::arg_list => {
                args = child
                    .into_inner()
                    .filter(|arg| arg.as_rule() == Rule::expr)
                    .map(|arg| expr::build_value_expr(arg))
                    .collect::<Result<Vec<_>, _>>()?;
            }
            Rule::yield_clause => yield_items = build_yield_items(child)?,
            Rule::yield_filter => {
                yield_filter = Some(expr::build_value_expr(first_child(child)?)?);
            }
            _ => return Err(unexpected_pair(child, "unexpected procedure-call child")),
        }
    }

    Ok(ProcedureCall {
        optional: false,
        name: NonEmpty::try_from_vec(name.ok_or_else(|| {
            ParserError::syntax("procedure call is missing name", source_span, None)
        })?)
        .expect("grammar guarantees >= 1: qualified_name"),
        args,
        yield_items,
        yield_filter,
        span: source_span,
    })
}

pub(super) fn build_yield_items(pair: Pair<'_, Rule>) -> Result<Vec<YieldItem>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::yield_item)
        .map(|child| build_yield_item(child))
        .collect()
}

fn build_yield_item(pair: Pair<'_, Rule>) -> Result<YieldItem, ParserError> {
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
                column = Some(YieldColumn::Named(db_string_pair(child)?));
            }
            Rule::alias => alias = Some(db_string_pair(first_child(child)?)?),
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
