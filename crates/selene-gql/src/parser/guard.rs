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
/// pest's recursive descent recurses one stack frame per nesting level. Two
/// `grammar.pest` rules recurse back toward `expr` with **no** guarded
/// delimiter (`(`/`[`/`{`) in between, so [`MAX_NESTING_DEPTH`] (which only
/// counts delimiters) cannot bound them:
///
/// - `unary = { sign_op ~ unary | postfix }` (`grammar.pest:380`) — a run of
///   leading unary `+`/`-` signs.
/// - `not_expr = { not_kw ~ not_expr | is_expr }` (`grammar.pest:324`) — a run
///   of `NOT` keywords. `NOT` is a reserved keyword (`grammar.pest` `keyword`
///   set), so it cannot be a *bare* identifier; in the few `prop_ident`/param
///   positions it can occupy (`n.NOT`, `{NOT: …}`, `$NOT`) the run counter is
///   reset by the surrounding `.`/`$`/`:`/separator before it can accrue, so
///   only a genuine consecutive `NOT NOT NOT` keyword chain reaches the cap.
///
/// (The keyword-bracketed `case_expr` ↔ `expr` recursion and the `type_name`
/// `LIST<…>` recursion are *also* zero-delimiter, but `CASE`/`END`/`LIST` are
/// legal identifiers in many context-sensitive positions — property names, map
/// keys, aliases, parameters, and — for the non-reserved `LIST` — bare variables
/// and `LIST < x` comparisons — which a byte scan cannot soundly distinguish
/// from the keyword. Bounding those recursions is deferred to a follow-up that
/// classifies tokens with the grammar's exact lexical rules. Until then they are
/// bounded only by the `stacker` parse backstop in `parser/mod.rs`.)
///
/// A long enough zero-delimiter run overflows the native stack inside pest.
/// A Rust stack overflow is **non-unwindable** (`catch_unwind` cannot trap it),
/// so a small hostile string would hard-kill the host process embedding
/// selene-db. This pre-pest cap rejects such runs deterministically before
/// recursive descent begins, mapped to GQLSTATUS `5GQL1`.
///
/// **This is a single COMBINED ceiling on total recursion pressure**, not a
/// per-counter cap. The native stack depth at the deepest point is the *sum* of
/// every simultaneously-open recursion frame — open delimiters (`depth`) *plus*
/// the current unary-sign chain *plus* the `NOT` chain — because they nest as
/// operands of one another (`(` `-` `(` `-` … each frame stays open until its
/// operand reduces). Capping each counter independently would admit their
/// *product* (e.g. 64 nested `(` each carrying a 255-deep sign chain ≈ 16 k
/// frames) while every individual counter stayed under its cap — a real
/// stack-overflow vector. So the guard bounds the running SUM
/// `depth + sign_run + not_run`, and the zero-token counters are reset only at a
/// value/closer (where the active chain reduces), **never** at an opener (where
/// the outer chain stays frozen-open and must keep counting). The delimiter caps
/// [`MAX_NESTING_DEPTH`] (64) and [`MAX_LIST_NESTING_DEPTH`] (32) remain as
/// tighter, more precise sub-bounds.
///
/// The cap mirrors `ANALYZER_MAX_DEPTH = 256` (`analyze/bind/mod.rs:29`) and is
/// a deliberate safety floor, not a tuning knob: it sits ~4-8x below the
/// smallest realistic stack's overflow point (a 2 MB nextest thread overflows
/// the sign chain at ~1000-2000) so regression tests can replay an over-cap
/// input without crashing the test runner, while the deepest legitimate nesting
/// of any kind anywhere in the workspace is 3.
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
    // Recursion-pressure counters (see `MAX_RECURSION_DEPTH`). Their SUM with
    // `depth` is the bounded quantity: it tracks the native stack depth at the
    // current position. pest treats comments as whitespace, so a comment between
    // two signs does NOT break the unary run (`- /* c */ -` is a 2-deep `unary`).
    // Resets happen ONLY at a value (a primary — string/backtick/number/other
    // word) or a closer (`)`/`}`/`]`), where the active unary/`NOT` chain
    // reduces; an OPENER (`(`/`{`/`[`/sign/`NOT`) never resets the other counter,
    // so a chain frozen open across a delimiter keeps being counted (otherwise
    // the guard would admit the product of the per-counter caps — see
    // `MAX_RECURSION_DEPTH`). Whitespace/comment arms `continue` without touching
    // any counter.
    let mut sign_run = 0_u32;
    let mut not_run = 0_u32;

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
            // An opener does NOT reset the sign / `NOT` runs: the delimiter
            // expression is the operand of any enclosing unary chain, which
            // stays open and must keep counting toward the combined cap.
            b'(' | b'{' => {
                depth += 1;
                if depth > MAX_NESTING_DEPTH {
                    return Err(ParserError::NestingLimitExceeded {
                        limit: MAX_NESTING_DEPTH,
                        span: point_span(index),
                    });
                }
                if exceeds_recursion_budget(depth, sign_run, not_run) {
                    return Err(ParserError::ComplexityLimitExceeded {
                        limit: MAX_RECURSION_DEPTH,
                        span: point_span(index),
                    });
                }
            }
            b'[' => {
                // `[` is the demonstrated super-linear backtracking vector, so
                // it carries the tighter dedicated depth cap on top of the
                // shared nesting cap. Check the tighter cap first so a deeply
                // nested `[` run is reported as the more precise complexity
                // violation rather than a generic nesting violation. Like the
                // other openers it does NOT reset the sign / `NOT` runs.
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
                if exceeds_recursion_budget(depth, sign_run, not_run) {
                    return Err(ParserError::ComplexityLimitExceeded {
                        limit: MAX_RECURSION_DEPTH,
                        span: point_span(index),
                    });
                }
            }
            // A closer reduces the delimiter expression, so the unary / `NOT`
            // chain that wrapped it (if any) is now complete: reset both runs.
            b')' | b'}' => {
                depth = depth.saturating_sub(1);
                sign_run = 0;
                not_run = 0;
            }
            b']' => {
                depth = depth.saturating_sub(1);
                list_depth = list_depth.saturating_sub(1);
                sign_run = 0;
                not_run = 0;
            }
            // A leading unary `+`/`-` opens one more `unary` frame. `--` is two
            // signs (not a comment); `->`/`-[r]-` and binary `a-b-c` never run
            // beyond ~2-3 before a value resets the run. It is an opener, so it
            // does not reset `not_run`; the combined budget bounds the depth.
            b'+' | b'-' => {
                sign_run += 1;
                if exceeds_recursion_budget(depth, sign_run, not_run) {
                    return Err(ParserError::ComplexityLimitExceeded {
                        limit: MAX_RECURSION_DEPTH,
                        span: point_span(index),
                    });
                }
            }
            // Whitespace is transparent to all counters (pest skips it).
            b' ' | b'\t' | b'\r' | b'\n' => {}
            // An ASCII identifier-start byte begins a whole word. `NOT` (the only
            // keyword the guard counts) opens a `not_expr` frame; every other
            // word is a value/primary that reduces the active unary / `NOT` chain.
            byte if is_word_start(byte) => {
                let (is_not_keyword, next_index) = recognize_not_keyword(bytes, index);
                index = next_index;
                if is_not_keyword {
                    not_run += 1;
                    if exceeds_recursion_budget(depth, sign_run, not_run) {
                        return Err(ParserError::ComplexityLimitExceeded {
                            limit: MAX_RECURSION_DEPTH,
                            span: point_span(index),
                        });
                    }
                } else {
                    sign_run = 0;
                    not_run = 0;
                }
                // `recognize_not_keyword` already advanced the index past the
                // word, so skip the trailing `index += 1` below.
                continue;
            }
            // Any other byte (operators like `.`, `,`, `*`, `<`, `>`, digits,
            // etc.) is a primary or separator: it reduces the active sign / `NOT`
            // chain.
            _ => {
                sign_run = 0;
                not_run = 0;
            }
        }
        index += 1;
    }

    Ok(())
}

/// Whether the combined recursion pressure exceeds [`MAX_RECURSION_DEPTH`].
///
/// The native pest stack depth at any position is the sum of every open
/// recursion frame: open delimiters (`depth`) plus the active unary-sign chain
/// (`sign_run`) plus the `NOT` chain (`not_run`). Bounding the SUM — not each
/// counter independently — is what prevents the product-depth vector (e.g.
/// nested `(` each carrying a long sign chain). `saturating_add` guards against
/// overflow even though each addend is checked at +1 increments.
fn exceeds_recursion_budget(depth: u32, sign_run: u32, not_run: u32) -> bool {
    depth.saturating_add(sign_run).saturating_add(not_run) > MAX_RECURSION_DEPTH
}

fn next_is(bytes: &[u8], index: usize, expected: u8) -> bool {
    bytes.get(index + 1).is_some_and(|value| *value == expected)
}

/// An ASCII byte that may begin a GQL identifier word (`[A-Za-z_]`).
///
/// Mirrors the leading character class of `grammar.pest`'s identifier rules so
/// the guard segments words the way pest does for the keyword check. Non-ASCII
/// bytes never begin the keyword `NOT`, so they are handled by the catch-all
/// arm (which resets the runs — the conservative direction).
fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

/// An ASCII byte that may continue a GQL identifier word (`[A-Za-z0-9_]`).
fn is_word_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Consume one whole ASCII identifier word starting at `start` and report
/// whether it is exactly the `NOT` keyword.
///
/// Returns `(is_not, index_after_word)` so the caller can resume scanning past
/// it. Word boundaries match the grammar's `!(LETTER | NUMBER | "_")` keyword
/// guard: `NOTE`/`NOTNULL` are distinct words and do not match `NOT`. Comparison
/// is ASCII-case-insensitive, matching the `^"NOT"` case-insensitive rule. (A
/// `NOT` used as a property/parameter name is harmless: the run counter is reset
/// by the surrounding `.`/`$`/`:`/separator before it can accrue.)
fn recognize_not_keyword(bytes: &[u8], start: usize) -> (bool, usize) {
    let mut end = start;
    while end < bytes.len() && is_word_continue(bytes[end]) {
        end += 1;
    }
    (bytes[start..end].eq_ignore_ascii_case(b"NOT"), end)
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
