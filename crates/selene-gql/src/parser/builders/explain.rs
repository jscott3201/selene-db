//! EXPLAIN statement builder.

use pest::iterators::Pair;

use crate::{Statement, error::ParserError};

use super::{Rule, build_statement, first_child, span};

pub(super) fn build_explain_statement(pair: Pair<'_, Rule>) -> Result<Statement, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::explain_stmt);
    let source_span = span(&pair);
    let inner = pair
        .into_inner()
        .find(|child| child.as_rule() == Rule::explainable_statement)
        .ok_or_else(|| {
            ParserError::syntax("EXPLAIN is missing an inner statement", source_span, None)
        })?;
    let inner = first_child(inner)?;
    let inner = build_statement(inner)?;
    Ok(Statement::Explain {
        inner: Box::new(inner),
        span: source_span,
    })
}
