//! Pair-to-AST builders.

pub(super) mod call;
pub(super) mod ddl;
pub(super) mod explain;
pub(super) mod expr;
pub(super) mod mutation;
pub(super) mod pattern;
pub(super) mod transaction;

use pest::iterators::Pair;
use selene_core::IStr;

use crate::{
    ast::{
        GqlType, LetBinding, LimitValue, NullsPolicy, OrderDirection, OrderTerm, PipelineStatement,
        QueryPipeline, ReturnClause, ReturnItem, SetOp, SourceSpan, Statement, UnwindStatement,
        WithClause, util::NonEmpty,
    },
    error::ParserError,
};

use super::{Rule, budget::InternerBudget};

pub(crate) fn build_statement(
    program_pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<Statement, ParserError> {
    match program_pair.as_rule() {
        Rule::gql_program => {
            let child = first_non_eoi(program_pair)?;
            build_statement(child, budget)
        }
        Rule::query_pipeline => build_query_pipeline(program_pair, budget).map(Statement::Query),
        Rule::composite_query => build_composite(program_pair, budget),
        Rule::chained_query => build_chained(program_pair, budget),
        Rule::pipeline_statement => {
            let span = span(&program_pair);
            let statement = build_pipeline_statement(program_pair, budget)?;
            Ok(Statement::Query(QueryPipeline {
                statements: vec![statement],
                span,
            }))
        }
        Rule::select_stmt => build_select_pipeline(program_pair, budget).map(Statement::Query),
        Rule::mutation_pipeline => {
            mutation::build_mutation_pipeline(program_pair, budget).map(Statement::Mutate)
        }
        Rule::ddl_statement => ddl::build_ddl_statement(program_pair, budget).map(Statement::Ddl),
        Rule::call_stmt => call::build_top_level_call(program_pair, budget),
        Rule::explain_stmt => explain::build_explain_statement(program_pair, budget),
        Rule::transaction_control => transaction::build_transaction_control(program_pair),
        _ => Err(unexpected_pair(program_pair, "expected a GQL program")),
    }
}

fn build_composite(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<Statement, ParserError> {
    let source_span = span(&pair);
    let mut children = pair.into_inner();
    let first = children
        .next()
        .ok_or_else(ParserError::empty_program)
        .and_then(|pair| build_query_pipeline(pair, budget))?;
    let mut rest = Vec::new();

    while let Some(op_pair) = children.next() {
        let op = build_set_op(op_pair)?;
        let pipeline = children
            .next()
            .ok_or_else(ParserError::empty_program)
            .and_then(|pair| build_query_pipeline(pair, budget))?;
        rest.push((op, pipeline));
    }

    Ok(Statement::Composite {
        first,
        rest: NonEmpty::try_from_vec(rest)
            .expect("grammar guarantees >= 1: composite_query set operator"),
        span: source_span,
    })
}

fn build_chained(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<Statement, ParserError> {
    let source_span = span(&pair);
    let blocks = pair
        .into_inner()
        .filter(|child| child.as_rule() == Rule::query_pipeline)
        .map(|child| build_query_pipeline(child, budget))
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

pub(super) fn build_query_pipeline(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<QueryPipeline, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::query_pipeline);
    let source_span = span(&pair);
    let statements = pair
        .into_inner()
        .map(|child| build_pipeline_statement(child, budget))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(QueryPipeline {
        statements,
        span: source_span,
    })
}

fn build_pipeline_statement(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<PipelineStatement, ParserError> {
    if pair.as_rule() == Rule::pipeline_statement {
        return build_pipeline_statement(first_child(pair)?, budget);
    }

    match pair.as_rule() {
        Rule::match_stmt => pattern::build_match_clause(pair, budget).map(PipelineStatement::Match),
        Rule::filter_stmt => build_filter(pair, budget).map(PipelineStatement::Filter),
        Rule::let_stmt => build_let(pair, budget).map(PipelineStatement::Let),
        Rule::unwind_stmt => build_unwind(pair, budget).map(PipelineStatement::Unwind),
        Rule::sorting_stmt => build_sorting(pair, budget).map(PipelineStatement::Sorting),
        Rule::offset_stmt => build_limit_or_offset(pair, budget).map(PipelineStatement::Offset),
        Rule::limit_stmt => build_limit_or_offset(pair, budget).map(PipelineStatement::Limit),
        Rule::return_stmt => build_return_clause(pair, budget).map(PipelineStatement::Return),
        Rule::with_stmt => build_with_clause(pair, budget).map(PipelineStatement::With),
        Rule::for_stmt => Err(not_implemented(&pair, "FOR is not yet supported in v1.0")),
        Rule::call_stmt => call::build_pipeline_call(pair, budget),
        _ => Err(unexpected_pair(pair, "expected pipeline statement")),
    }
}

fn build_select_pipeline(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<QueryPipeline, ParserError> {
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
            Rule::projection_list => return_clause.items = build_projection_list(child, budget)?,
            Rule::select_from => {
                let from_child = first_child(child)?;
                if from_child.as_rule() == Rule::match_stmt {
                    pre_return.push(PipelineStatement::Match(pattern::build_match_clause(
                        from_child, budget,
                    )?));
                } else {
                    return Err(not_implemented(
                        &from_child,
                        "SELECT FROM graph-name resolution is not yet supported in v1.0",
                    ));
                }
            }
            Rule::where_clause => {
                pre_return.push(PipelineStatement::Filter(build_where(child, budget)?));
            }
            Rule::group_by_clause => return_clause.group_by = Some(build_group_by(child, budget)?),
            Rule::having_clause => return_clause.having = Some(build_having(child, budget)?),
            Rule::sorting_stmt => {
                post_return.push(PipelineStatement::Sorting(build_sorting(child, budget)?));
            }
            Rule::offset_stmt => {
                post_return.push(PipelineStatement::Offset(build_limit_or_offset(
                    child, budget,
                )?));
            }
            Rule::limit_stmt => {
                post_return.push(PipelineStatement::Limit(build_limit_or_offset(
                    child, budget,
                )?));
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

pub(super) fn build_filter(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<crate::ast::ValueExpr, ParserError> {
    expr_from_first(pair, "FILTER is missing expression", budget)
}

fn build_where(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<crate::ast::ValueExpr, ParserError> {
    expr_from_first(pair, "WHERE is missing expression", budget)
}

fn build_having(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<crate::ast::ValueExpr, ParserError> {
    expr_from_first(pair, "HAVING is missing expression", budget)
}

fn expr_from_first(
    pair: Pair<'_, Rule>,
    missing: &'static str,
    budget: &mut InternerBudget,
) -> Result<crate::ast::ValueExpr, ParserError> {
    let pair_span = span(&pair);
    let expr_pair = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::expr)
        .ok_or_else(|| ParserError::syntax(missing, pair_span, None))?;
    expr::build_value_expr(expr_pair, budget)
}

fn build_let(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<Vec<LetBinding>, ParserError> {
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
                alias: intern_pair(alias_pair, budget)?,
                value: expr::build_value_expr(value_pair, budget)?,
                span: binding_span,
            })
        })
        .collect()
}

fn build_unwind(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<UnwindStatement, ParserError> {
    let source_span = span(&pair);
    let mut children = pair.into_inner();
    let source = children
        .next()
        .ok_or_else(|| {
            ParserError::syntax("UNWIND is missing source expression", source_span, None)
        })
        .and_then(|pair| expr::build_value_expr(pair, budget))?;
    let alias = children
        .next()
        .ok_or_else(|| ParserError::syntax("UNWIND is missing alias", source_span, None))
        .and_then(|pair| intern_pair(pair, budget))?;
    Ok(UnwindStatement {
        source,
        alias,
        span: source_span,
    })
}

fn build_sorting(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<Vec<OrderTerm>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::order_term)
        .map(|child| build_order_term(child, budget))
        .collect()
}

fn build_order_term(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<OrderTerm, ParserError> {
    let source_span = span(&pair);
    let mut expr_value = None;
    let mut direction = OrderDirection::Asc;
    let mut nulls = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::expr => expr_value = Some(expr::build_value_expr(child, budget)?),
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

fn build_limit_or_offset(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<LimitValue, ParserError> {
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
        Rule::typed_param_ref => {
            let (name, declared_type, span) = build_typed_param_ref(inner, budget)?;
            Ok(LimitValue::Parameter {
                name,
                declared_type,
                span,
            })
        }
        _ => Err(unexpected_pair(inner, "expected LIMIT/OFFSET value")),
    }
}

pub(super) fn build_return_clause(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<ReturnClause, ParserError> {
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
            Rule::projection_list => clause.items = build_projection_list(child, budget)?,
            Rule::group_by_clause => clause.group_by = Some(build_group_by(child, budget)?),
            Rule::having_clause => clause.having = Some(build_having(child, budget)?),
            Rule::no_bindings => {
                return Err(not_implemented(
                    &child,
                    "RETURN NO BINDINGS is not yet supported in v1.0",
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

fn build_with_clause(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<WithClause, ParserError> {
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
            Rule::projection_list => clause.items = build_projection_list(child, budget)?,
            Rule::group_by_clause => clause.group_by = Some(build_group_by(child, budget)?),
            Rule::having_clause => clause.having = Some(build_having(child, budget)?),
            Rule::where_clause => clause.where_clause = Some(build_where(child, budget)?),
            _ => return Err(unexpected_pair(child, "unexpected WITH child")),
        }
    }
    Ok(clause)
}

fn build_projection_list(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<Vec<ReturnItem>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::projection)
        .map(|child| build_return_item(child, budget))
        .collect()
}

fn build_return_item(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<ReturnItem, ParserError> {
    let source_span = span(&pair);
    let mut value = None;
    let mut alias = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::expr => value = Some(expr::build_value_expr(child, budget)?),
            Rule::alias => alias = Some(build_alias(child, budget)?),
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

fn build_alias(pair: Pair<'_, Rule>, budget: &mut InternerBudget) -> Result<IStr, ParserError> {
    let source_span = span(&pair);
    let ident = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::prop_ident)
        .ok_or_else(|| ParserError::syntax("AS alias is missing identifier", source_span, None))?;
    intern_pair(ident, budget)
}

fn build_group_by(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<Vec<crate::ast::ValueExpr>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::group_by_item)
        .map(|item| expr_from_first(item, "GROUP BY item is missing expression", budget))
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

pub(super) fn intern_pair(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<IStr, ParserError> {
    let source_span = span(&pair);
    let decoded = decode_ident_like(pair.as_str());
    budget.intern_str(&decoded, source_span, "identifier")
}

/// Build a qualified name as a list of interned segments.
///
/// Each grammar segment is interned independently. Quoted segments containing
/// dots stay one segment, so `foo."bar.baz"` and `foo.bar.baz` produce
/// different paths.
pub(super) fn build_qualified_name(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<Vec<IStr>, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::qualified_name);
    let source_span = span(&pair);
    let mut segments = Vec::new();
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::ident | Rule::prop_ident => {
                let canonical = decode_ident_like(child.as_str());
                let interned =
                    budget.intern_str(&canonical, source_span, "qualified-name segment")?;
                segments.push(interned);
            }
            _ => return Err(unexpected_pair(child, "unexpected qualified-name child")),
        }
    }
    if segments.is_empty() {
        return Err(ParserError::syntax(
            "qualified name has no segments",
            source_span,
            None,
        ));
    }
    Ok(segments)
}

pub(super) fn intern_param(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<IStr, ParserError> {
    let source_span = span(&pair);
    let text = pair.as_str().strip_prefix('$').unwrap_or(pair.as_str());
    budget.intern_str(text, source_span, "parameter")
}

pub(super) fn build_typed_param_ref(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<(IStr, Option<GqlType>, SourceSpan), ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::typed_param_ref);
    let source_span = span(&pair);
    let mut name = None;
    let mut param_span = None;
    let mut declared_type = None;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::param_ref => {
                param_span = Some(span(&child));
                name = Some(intern_param(child, budget)?);
            }
            Rule::type_name => declared_type = Some(expr::build_type_name(child)?),
            _ => return Err(unexpected_pair(child, "unexpected typed parameter child")),
        }
    }
    let name = name.ok_or_else(|| {
        ParserError::syntax(
            "typed parameter reference is missing name",
            source_span,
            None,
        )
    })?;
    let source_span = if declared_type.is_some() {
        source_span
    } else {
        param_span.unwrap_or(source_span)
    };
    Ok((name, declared_type, source_span))
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
        Some(
            "this construct is not yet supported in v1.0; use `CALL selene.feature_status()` for feature status or `SHOW PROCEDURES` for registered procedures",
        ),
    )
}
