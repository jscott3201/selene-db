//! Resolve quote ambiguity with the existing parse, never a speculative parse.

use pest::iterators::Pairs;

use super::{ParserError, validate_with_quotes};
use crate::parser::Rule;

/// Candidate shared-retry rejection; hard stack guards have already run.
pub(in crate::parser) struct RetryCheck(pub(in crate::parser) Option<ParserError>);

/// Run all hard guards, but leave the shared retry decision to the parser.
pub(in crate::parser) fn validate(source: &str) -> Result<RetryCheck, ParserError> {
    validate_with_quotes(source, |_| None)
}

/// Reuse the successful parse's token boundaries before building any AST.
///
/// In expression position a quoted name may be a function root or a string.
/// Only the complete ordered-choice parse decides which: even a following `(`
/// is insufficient. Flattening the existing Pair queue is iterative and the
/// peekable iterator retains just one span, not an input-sized token copy.
pub(in crate::parser) fn validate_parsed(
    source: &str,
    pairs: Pairs<'_, Rule>,
) -> Result<(), ParserError> {
    let mut quotes = pairs
        .flatten()
        .filter(|pair| {
            matches!(
                pair.as_rule(),
                Rule::ident | Rule::prop_ident | Rule::string_lit
            ) && matches!(pair.as_str().as_bytes().first(), Some(b'"' | b'`'))
        })
        .map(|pair| pair.as_span())
        .peekable();
    let check = validate_with_quotes(source, |index| {
        while quotes.peek().is_some_and(|span| span.start() < index) {
            quotes.next();
        }
        quotes
            .peek()
            .filter(|span| span.start() == index)
            .map(|span| span.end() - 1)
    })?;
    check.0.map_or(Ok(()), Err)
}
