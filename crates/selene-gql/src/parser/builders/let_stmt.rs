//! LET statement builder.

use pest::iterators::Pair;

use crate::{ast::LetBinding, error::ParserError};

use super::{Rule, db_string_pair, expr, first_child, span, unexpected_pair};

pub(super) fn build_let(pair: Pair<'_, Rule>) -> Result<Vec<LetBinding>, ParserError> {
    pair.into_inner()
        .filter(|child| child.as_rule() == Rule::let_binding)
        .map(build_let_binding)
        .collect()
}

fn build_let_binding(pair: Pair<'_, Rule>) -> Result<LetBinding, ParserError> {
    let binding_span = span(&pair);
    let inner = first_child(pair)?;
    match inner.as_rule() {
        Rule::let_value_binding => build_let_value_binding(inner),
        Rule::let_shorthand_binding => build_let_shorthand_binding(inner),
        _ => Err(unexpected_pair(inner, "unexpected LET binding child")),
    }
    .map(|mut binding| {
        binding.span = binding_span;
        binding
    })
}

fn build_let_value_binding(pair: Pair<'_, Rule>) -> Result<LetBinding, ParserError> {
    let binding_span = span(&pair);
    let mut alias = None;
    let mut declared_type = None;
    let mut value = None;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::value_type_kw => {}
            Rule::ident => alias = Some(db_string_pair(child)?),
            Rule::let_value_declared_type => {
                let type_pair = child
                    .into_inner()
                    .find(|inner| inner.as_rule() == Rule::type_name)
                    .ok_or_else(|| {
                        ParserError::syntax(
                            "LET VALUE declared type is missing type name",
                            binding_span,
                            None,
                        )
                    })?;
                declared_type = Some(expr::build_type_name(type_pair)?);
            }
            Rule::expr => value = Some(expr::build_value_expr(child)?),
            _ => return Err(unexpected_pair(child, "unexpected LET VALUE child")),
        }
    }
    Ok(LetBinding {
        alias: alias.ok_or_else(|| {
            ParserError::syntax("LET binding is missing alias", binding_span, None)
        })?,
        declared_type,
        value: value.ok_or_else(|| {
            ParserError::syntax("LET binding is missing expression", binding_span, None)
        })?,
        span: binding_span,
    })
}

fn build_let_shorthand_binding(pair: Pair<'_, Rule>) -> Result<LetBinding, ParserError> {
    let binding_span = span(&pair);
    let mut children = pair.into_inner();
    let alias_pair = children
        .next()
        .ok_or_else(|| ParserError::syntax("LET binding is missing alias", binding_span, None))?;
    let value_pair = children.next().ok_or_else(|| {
        ParserError::syntax("LET binding is missing expression", binding_span, None)
    })?;
    Ok(LetBinding {
        alias: db_string_pair(alias_pair)?,
        declared_type: None,
        value: expr::build_value_expr(value_pair)?,
        span: binding_span,
    })
}
