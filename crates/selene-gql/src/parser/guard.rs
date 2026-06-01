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

/// Maximum zero-delimiter recursive-descent depth admitted by the parser.
///
/// pest's recursive descent recurses one stack frame per nesting level. Three
/// `grammar.pest` rules recurse back toward `expr` with **no** guarded
/// delimiter (`(`/`[`/`{`) in between, so [`MAX_NESTING_DEPTH`] (which only
/// counts delimiters) cannot bound them:
///
/// - `unary = { sign_op ~ unary | postfix }` (`grammar.pest:380`) — a run of
///   leading unary `+`/`-` signs.
/// - `not_expr = { not_kw ~ not_expr | is_expr }` (`grammar.pest:324`) — a run
///   of `NOT` keywords.
/// - `case_expr` ↔ `expr` (`grammar.pest:447-452`) — `^"CASE" ~ expr` nests a
///   fresh expression with only the `CASE` keyword before it.
///
/// A long enough zero-delimiter run overflows the native stack inside pest.
/// A Rust stack overflow is **non-unwindable** (`catch_unwind` cannot trap it),
/// so a small hostile string would hard-kill the host process embedding
/// selene-db. This pre-pest cap rejects such runs deterministically before
/// recursive descent begins, mapped to GQLSTATUS `5GQL1`.
///
/// The cap mirrors `ANALYZER_MAX_DEPTH = 256` (`analyze/bind/mod.rs:29`) and is
/// a deliberate safety floor, not a tuning knob: it sits ~4-8x below the
/// smallest realistic stack's overflow point (a 2 MB nextest thread overflows
/// the sign chain at ~1000-2000) so regression tests can replay an over-cap
/// input without crashing the test runner, while the deepest legitimate sign /
/// `NOT` / `CASE` nesting anywhere in the workspace is 3.
pub(crate) const MAX_RECURSION_DEPTH: u32 = 256;

pub(super) fn validate(source: &str) -> Result<(), ParserError> {
    let bytes = source.as_bytes();
    // Index of the final `'` in the input. A single-quoted string treats `\'`
    // as an escaped quote ONLY when a later `'` exists (pest `escaped_quote`);
    // when the `'` after a `\` is the last quote, the `\` is a dangling escape
    // and the `'` closes the string (pest `dangling_escape`). Mirroring this is
    // load-bearing: a blanket "`\` escapes the next byte" rule lets `RETURN '\'`
    // skip past the real closing quote to EOF, hiding a hostile `[` run that
    // pest still closes the string before and parses — a parser-time DoS bypass.
    let last_single_quote = bytes.iter().rposition(|byte| *byte == b'\'');
    let mut index = 0;
    let mut depth = 0_u32;
    let mut list_depth = 0_u32;
    // Zero-delimiter recursion counters (see `MAX_RECURSION_DEPTH`). pest treats
    // comments as whitespace, so a comment between two signs does NOT break the
    // unary run (`- /* c */ -` is a 2-deep `unary`); a string / backtick literal
    // is a primary and DOES reset the run. The `skip_*` helpers below advance
    // past each span; whitespace/comment arms therefore must `continue` WITHOUT
    // resetting `sign_run`, while every primary-introducing byte resets it.
    let mut sign_run = 0_u32;
    let mut not_run = 0_u32;
    let mut case_depth = 0_u32;

    while index < bytes.len() {
        match bytes[index] {
            // String / backtick literals are primaries: they reset the unary
            // and `NOT` runs (a primary terminates a leading-sign / `NOT` chain),
            // then the scan resumes after the closing quote.
            b'\'' => {
                sign_run = 0;
                not_run = 0;
                index = skip_single_quoted(bytes, index + 1, last_single_quote);
            }
            b'"' => {
                sign_run = 0;
                not_run = 0;
                index = skip_double_quoted(bytes, index + 1);
            }
            b'`' => {
                sign_run = 0;
                not_run = 0;
                index = skip_backtick_quoted(bytes, index + 1);
            }
            // Comments are whitespace to pest, so they do NOT reset any run:
            // `- // c\n -` is still a 2-deep unary chain. Skip the span and
            // continue without touching the counters. `--` is two unary minus
            // `sign_op` (`grammar.pest:642` defines `//` as the only line
            // comment), so it is handled by the `b'-'` arm, NOT skipped here.
            b'/' if next_is(bytes, index, b'/') => {
                index = skip_line_comment(bytes, index + 2);
                continue;
            }
            b'/' if next_is(bytes, index, b'*') => {
                index = skip_block_comment(bytes, index + 2);
                index += 1;
                continue;
            }
            b'(' | b'{' => {
                sign_run = 0;
                not_run = 0;
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
                sign_run = 0;
                not_run = 0;
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
            // A leading unary `+`/`-` extends the `unary` chain. `--` is two
            // signs (not a comment), `-->`/`<--`/`-[r]-` and binary `a-b-c`
            // never exceed run ~2-3. A sign does not terminate a `NOT` chain in
            // a way the count cares about, but it is not a `NOT`, so reset
            // `not_run`. The cap bounds the maximal consecutive run.
            b'+' | b'-' => {
                not_run = 0;
                sign_run += 1;
                if sign_run > MAX_RECURSION_DEPTH {
                    return Err(ParserError::ComplexityLimitExceeded {
                        limit: MAX_RECURSION_DEPTH,
                        span: point_span(index),
                    });
                }
            }
            // Whitespace is transparent to all three runs (pest skips it).
            b' ' | b'\t' | b'\r' | b'\n' => {}
            // An ASCII identifier-start byte begins a whole word. Recognize the
            // word case-insensitively: `NOT` (and `CASE`/`END`) drive the
            // keyword-only recursion counters; any other word is a primary that
            // resets the unary and `NOT` runs.
            byte if is_word_start(byte) => {
                let (word_kind, next_index) = recognize_word(bytes, index);
                index = next_index;
                match word_kind {
                    WordKind::Not => {
                        sign_run = 0;
                        not_run += 1;
                        if not_run > MAX_RECURSION_DEPTH {
                            return Err(ParserError::ComplexityLimitExceeded {
                                limit: MAX_RECURSION_DEPTH,
                                span: point_span(index),
                            });
                        }
                    }
                    WordKind::Case => {
                        sign_run = 0;
                        not_run = 0;
                        case_depth += 1;
                        if case_depth > MAX_RECURSION_DEPTH {
                            return Err(ParserError::ComplexityLimitExceeded {
                                limit: MAX_RECURSION_DEPTH,
                                span: point_span(index),
                            });
                        }
                    }
                    WordKind::End => {
                        sign_run = 0;
                        not_run = 0;
                        case_depth = case_depth.saturating_sub(1);
                    }
                    WordKind::Other => {
                        sign_run = 0;
                        not_run = 0;
                    }
                }
                // `recognize_word` already advanced the index past the word, so
                // skip the trailing `index += 1` below.
                continue;
            }
            // Any other byte (operators like `.`, `,`, `*`, digits, etc.) is a
            // primary or separator: it terminates a leading-sign / `NOT` chain.
            _ => {
                sign_run = 0;
                not_run = 0;
            }
        }
        index += 1;
    }

    Ok(())
}

fn next_is(bytes: &[u8], index: usize, expected: u8) -> bool {
    bytes.get(index + 1).is_some_and(|value| *value == expected)
}

/// The whole-word categories the recursion counters care about.
#[derive(Clone, Copy, Eq, PartialEq)]
enum WordKind {
    /// `NOT` (case-insensitive, whole word) — drives `not_expr` recursion.
    Not,
    /// `CASE` (case-insensitive, whole word) — opens a `case_expr`.
    Case,
    /// `END` (case-insensitive, whole word) — closes a `case_expr`.
    End,
    /// Any other identifier word — a primary that resets the leading-sign /
    /// `NOT` runs.
    Other,
}

/// An ASCII byte that may begin a GQL identifier word (`[A-Za-z_]`).
///
/// Mirrors the leading character class of `grammar.pest`'s identifier rules so
/// the guard segments words exactly as pest does. Non-ASCII bytes never begin
/// the keywords `NOT`/`CASE`/`END`, so they are handled by the catch-all arm.
fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

/// An ASCII byte that may continue a GQL identifier word (`[A-Za-z0-9_]`).
fn is_word_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Consume one whole ASCII identifier word starting at `start` and classify it.
///
/// Returns the word category and the index of the first byte **after** the
/// word, so the caller can resume scanning past it (the keyword guards must not
/// re-scan the word's interior). Word boundaries match the grammar's
/// `!(LETTER | NUMBER | "_")` keyword guard: `NOTE`/`NOTNULL` are distinct words
/// and do not match `NOT`. Comparison is ASCII-case-insensitive, matching the
/// `^"..."` (case-insensitive literal) keyword rules.
fn recognize_word(bytes: &[u8], start: usize) -> (WordKind, usize) {
    let mut end = start;
    while end < bytes.len() && is_word_continue(bytes[end]) {
        end += 1;
    }
    let word = &bytes[start..end];
    let kind = if word.eq_ignore_ascii_case(b"NOT") {
        WordKind::Not
    } else if word.eq_ignore_ascii_case(b"CASE") {
        WordKind::Case
    } else if word.eq_ignore_ascii_case(b"END") {
        WordKind::End
    } else {
        WordKind::Other
    };
    (kind, end)
}

fn skip_single_quoted(bytes: &[u8], mut index: usize, last_quote: Option<usize>) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            // `\'` where the `'` is the final quote in the input is a *dangling*
            // escape (pest `dangling_escape`): the `\` is literal and the `'`
            // closes the string. Return the `'` position so the scan resumes
            // after it and still counts any following brackets — matching pest,
            // which closes the string here too. Any other `\X` (including a
            // `\'` with a later quote — pest `escaped_quote`) escapes one byte.
            b'\\' if bytes.get(index + 1) == Some(&b'\'') && Some(index + 1) == last_quote => {
                return index + 1;
            }
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
