//! Cheap parser input guards that run before pest recursive descent.

use crate::{SourceSpan, error::ParserError};

/// Maximum syntactic nesting depth admitted by the parser.
///
/// This bounds pest recursion on hostile malformed expressions while leaving
/// ordinary query, list, record, and subquery nesting comfortably below the cap.
pub(crate) const MAX_NESTING_DEPTH: u32 = 64;

/// Maximum total count of grouping, list, and record openers admitted in a
/// single statement, regardless of how deeply they nest.
///
/// pest is not packrat-memoized, so every failed sub-parse of an opener is
/// recomputed. The three `[`-prefixed expression rules
/// (`list_access_op`, `list_comprehension`, `list_lit`; see `grammar.pest`)
/// each re-explore a run of openers, so a wide, shallow fan-out of `[` (or
/// `(`/`{`) runs drives super-linear backtracking — seconds-to-minutes parse
/// time for sub-kilobyte hostile inputs — even when net nesting stays well
/// under [`MAX_NESTING_DEPTH`]. Because parsing precedes execution, an
/// execution deadline cannot interrupt it; the only safe place to stop the
/// blow-up is before recursive descent begins.
///
/// This is a conservative *complexity budget*, not an exact root-cause metric:
/// one observed 56-opener artifact parses fast and a 57-opener one is slow, so
/// the cap is set generously above the legitimate maximum rather than at the
/// empirical blow-up point. The largest single-statement opener count in the
/// positive corpus is 9 (a 9-argument trigonometric `RETURN`), and no
/// legitimate query anywhere in the workspace exceeds 9, so a cap of 20 leaves
/// comfortable headroom while rejecting every known hostile artifact (the
/// smallest of which uses 56 openers).
pub(crate) const MAX_BRACKET_OPENER_COUNT: u32 = 20;

pub(super) fn validate(source: &str) -> Result<(), ParserError> {
    let bytes = source.as_bytes();
    let mut index = 0;
    let mut depth = 0_u32;
    let mut openers = 0_u32;

    while index < bytes.len() {
        match bytes[index] {
            b'\'' => index = skip_single_quoted(bytes, index + 1),
            b'"' => index = skip_double_quoted(bytes, index + 1),
            b'`' => index = skip_backtick_quoted(bytes, index + 1),
            b'/' if next_is(bytes, index, b'/') => index = skip_line_comment(bytes, index + 2),
            b'-' if next_is(bytes, index, b'-') => index = skip_line_comment(bytes, index + 2),
            b'/' if next_is(bytes, index, b'*') => index = skip_block_comment(bytes, index + 2),
            b'(' | b'[' | b'{' => {
                // Bound total openers first: it is the tighter cap and the
                // primary defense against the wide fan-out blow-up. A balanced
                // input deep enough to exceed MAX_NESTING_DEPTH also exceeds
                // this cap, so it is reported as the (tighter, more honest)
                // complexity violation rather than a nesting violation.
                openers += 1;
                if openers > MAX_BRACKET_OPENER_COUNT {
                    return Err(ParserError::ComplexityLimitExceeded {
                        limit: MAX_BRACKET_OPENER_COUNT,
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
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
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
