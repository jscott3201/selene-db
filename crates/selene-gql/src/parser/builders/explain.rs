//! EXPLAIN statement builder.

use pest::iterators::Pair;

use crate::{Statement, error::ParserError, parser::budget::InternerBudget};

use super::{Rule, build_statement, first_child, span, unexpected_pair};

pub(super) fn build_explain_statement(
    pair: Pair<'_, Rule>,
    budget: &mut InternerBudget,
) -> Result<Statement, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::explain_stmt);
    let source_span = span(&pair);
    let inner = first_child(pair)?;
    let inner = match inner.as_rule() {
        Rule::explainable_statement => first_child(inner)?,
        _ => return Err(unexpected_pair(inner, "expected EXPLAIN inner statement")),
    };
    let inner = build_statement(inner, budget)?;
    Ok(Statement::Explain {
        inner: Box::new(inner),
        span: source_span,
    })
}
