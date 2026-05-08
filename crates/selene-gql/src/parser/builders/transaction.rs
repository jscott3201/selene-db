//! Transaction-control builders.

use pest::iterators::Pair;

use crate::{ast::Statement, error::ParserError};

use super::{Rule, first_child, span, unexpected_pair};

pub(super) fn build_transaction_control(pair: Pair<'_, Rule>) -> Result<Statement, ParserError> {
    debug_assert_eq!(pair.as_rule(), Rule::transaction_control);
    let inner = first_child(pair)?;
    let source_span = span(&inner);
    match inner.as_rule() {
        Rule::start_transaction => Ok(Statement::StartTransaction { span: source_span }),
        Rule::commit => Ok(Statement::Commit { span: source_span }),
        Rule::rollback => Ok(Statement::Rollback { span: source_span }),
        _ => Err(unexpected_pair(
            inner,
            "expected transaction-control statement",
        )),
    }
}
