//! Cheap parser input guards that run before pest recursive descent.

use crate::{SourceSpan, error::ParserError};

/// Maximum syntactic nesting depth admitted by the parser.
///
/// This bounds pest recursion on hostile malformed expressions while leaving
/// ordinary query, list, record, and subquery nesting comfortably below the cap.
pub(crate) const MAX_NESTING_DEPTH: u32 = 64;

/// Maximum simultaneously-open `[` (list / index / comprehension) nesting
/// depth admitted in a single statement.
///
/// pest is not packrat-memoized, so a `[` opener is re-explored by each of the
/// three `[`-prefixed expression rules (`list_access_op`, `list_comprehension`,
/// `list_lit`; see `grammar.pest`) before the parser can commit. A run of
/// *unclosed* `[` therefore nests those ambiguous sub-parses, and the failed
/// branches are recomputed at every level — super-linear backtracking that
/// reaches seconds-to-minutes parse time for sub-kilobyte hostile inputs (the
/// fuzz corpus blows up around 57 nested `[`), well under [`MAX_NESTING_DEPTH`].
/// Parsing precedes execution, so an execution deadline cannot interrupt it;
/// the only safe place to stop the blow-up is before recursive descent begins.
///
/// This caps the *depth* of simultaneously-open `[`, **not** a total opener
/// count, because depth is the actual blow-up driver. A balanced, promptly
/// closed bracket never nests: an edge pattern `-[:E]->` and a flat list
/// `[1, 2, 3]` each return to `[`-depth 0 before the next opener, so a path of
/// arbitrarily many hops or an arbitrarily wide list stays at `[`-depth 1 and
/// is always admitted — only genuinely nested `[` accrue depth. (A total-count
/// cap, by contrast, would reject a legitimate ten-hop fixed path purely for
/// its opener count.) The cap of 32 is an order of magnitude above the deepest
/// legitimate `[` nesting anywhere in the workspace (3, a `[[[1]]]` literal)
/// and comfortably below the ~57-deep empirical blow-up point. `(` and `{`
/// nesting is not a demonstrated backtracking vector (the fuzz corpus contains
/// only `[`) and is bounded by [`MAX_NESTING_DEPTH`] alone.
pub(crate) const MAX_LIST_NESTING_DEPTH: u32 = 32;

pub(super) fn validate(source: &str) -> Result<(), ParserError> {
    let bytes = source.as_bytes();
    let mut index = 0;
    let mut depth = 0_u32;
    let mut list_depth = 0_u32;

    while index < bytes.len() {
        match bytes[index] {
            b'\'' => index = skip_single_quoted(bytes, index + 1),
            b'"' => index = skip_double_quoted(bytes, index + 1),
            b'`' => index = skip_backtick_quoted(bytes, index + 1),
            b'/' if next_is(bytes, index, b'/') => index = skip_line_comment(bytes, index + 2),
            b'-' if next_is(bytes, index, b'-') => index = skip_line_comment(bytes, index + 2),
            b'/' if next_is(bytes, index, b'*') => index = skip_block_comment(bytes, index + 2),
            b'(' | b'{' => {
                depth += 1;
                if depth > MAX_NESTING_DEPTH {
                    return Err(ParserError::NestingLimitExceeded {
                        limit: MAX_NESTING_DEPTH,
                        span: point_span(index),
                    });
                }
            }
            b'[' => {
                // `[` is the demonstrated super-linear backtracking vector, so
                // it carries the tighter dedicated depth cap on top of the
                // shared nesting cap. Check the tighter cap first so a deeply
                // nested `[` run is reported as the more precise complexity
                // violation rather than a generic nesting violation.
                list_depth += 1;
                if list_depth > MAX_LIST_NESTING_DEPTH {
                    return Err(ParserError::ComplexityLimitExceeded {
                        limit: MAX_LIST_NESTING_DEPTH,
                        span: point_span(index),
                    });
                }
                depth += 1;
                if depth > MAX_NESTING_DEPTH {
                    return Err(ParserError::NestingLimitExceeded {
                        limit: MAX_NESTING_DEPTH,
                        span: point_span(index),
                    });
                }
            }
            b')' | b'}' => depth = depth.saturating_sub(1),
            b']' => {
                depth = depth.saturating_sub(1);
                list_depth = list_depth.saturating_sub(1);
            }
            _ => {}
        }
        index += 1;
    }

    Ok(())
}

fn next_is(bytes: &[u8], index: usize, expected: u8) -> bool {
    bytes.get(index + 1).is_some_and(|value| *value == expected)
}

fn skip_single_quoted(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'\'' if next_is(bytes, index, b'\'') => index += 2,
            b'\'' => return index,
            _ => index += 1,
        }
    }
    bytes.len()
}

fn skip_double_quoted(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'"' if next_is(bytes, index, b'"') => index += 2,
            b'"' => return index,
            _ => index += 1,
        }
    }
    bytes.len()
}

fn skip_backtick_quoted(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        if bytes[index] == b'`' {
            return index;
        }
        index += 1;
    }
    bytes.len()
}

fn skip_line_comment(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            return index;
        }
        index += 1;
    }
    bytes.len()
}

fn skip_block_comment(bytes: &[u8], mut index: usize) -> usize {
    while index + 1 < bytes.len() {
        if bytes[index] == b'*' && bytes[index + 1] == b'/' {
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

fn point_span(offset: usize) -> SourceSpan {
    SourceSpan::new(u32::try_from(offset).unwrap_or(u32::MAX), 1)
}
