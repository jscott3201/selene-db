//! Procedure-call builders.

use pest::iterators::Pair;

use crate::{
    ast::{ProcedureCall, YieldColumn, YieldItem},
    error::ParserError,
    parser::budget::InternerBudget,
};

use super::{
    Rule, build_qualified_name, expr, first_child, intern_pair, not_implemented, span,
    unexpected_pair,
};

pub(super) fn build_top_level_call(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<ProcedureCall, ParserError> {
    build_call_stmt(pair, budget)
}

pub(super) fn build_pipeline_call(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<ProcedureCall, ParserError> {
    build_call_stmt(pair, budget)
}

fn build_call_stmt(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<ProcedureCall, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::call_stmt);
    let inner = first_child(pair)?;
    match inner.as_rule() {
        Rule::call_procedure => build_procedure_call(inner, budget),
        Rule::call_subquery => Err(not_implemented(
            &inner,
            "CALL { ... } subquery form lands in a future brief",
        )),
        _ => Err(unexpected_pair(inner, "expected CALL body")),
    }
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
                return Err(not_implemented(&child, "YIELD WHERE filters land in M5b"));
            }
            _ => return Err(unexpected_pair(child, "unexpected procedure-call child")),
        }
    }

    Ok(ProcedureCall {
        name: name.ok_or_else(|| {
            ParserError::syntax("procedure call is missing name", source_span, None)
        })?,
        args,
        yield_items,
        span: source_span,
    })
}

fn build_yield_items(
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
