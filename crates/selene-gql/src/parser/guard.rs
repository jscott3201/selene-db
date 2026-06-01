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
/// - `type_name = { … | ^"LIST" ~ "<" ~ type_name ~ ">" | … }`
///   (`grammar.pest:508`) — the only `type_name` self-recursion (reachable via
///   `cast_expr`, `IS TYPED`, `:: type_name`, record fields). Its inter-level
///   delimiter is `<`, which is **also** `comp_op` (`<`/`>`/`<=`/`>=`/`<>`,
///   `grammar.pest:369`) and so cannot be counted as a bracket. We instead bound
///   the `LIST` *keyword* depth (increment per `LIST`, decrement per `>`); this
///   never accumulates on a comparison chain (the depth only rises inside a
///   `LIST<…>` whose contents are types, never comparisons) and is immune to a
///   comment inserted between `LIST` and `<`.
///
/// A long enough zero-delimiter run overflows the native stack inside pest.
/// A Rust stack overflow is **non-unwindable** (`catch_unwind` cannot trap it),
/// so a small hostile string would hard-kill the host process embedding
/// selene-db. This pre-pest cap rejects such runs deterministically before
/// recursive descent begins, mapped to GQLSTATUS `5GQL1`.
///
/// **This is a single COMBINED ceiling on total recursion pressure**, not a
/// per-counter cap. The native stack depth at the deepest point is the *sum* of
/// every simultaneously-open recursion frame across all families — open
/// delimiters *plus* the current unary-sign chain *plus* the `NOT` chain *plus*
/// open `CASE` bodies *plus* open `LIST<…>` levels — because they nest as
/// operands of one another (`(` `-` `(` `-` … each frame stays open until its
/// operand reduces). Capping each counter independently would admit their
/// *product* (e.g. 64 nested `(` each carrying a 255-deep sign chain ≈ 16 k
/// frames) while every individual counter stayed under its cap — a real
/// stack-overflow vector. So the guard bounds the running SUM
/// `depth + sign_run + not_run + case_depth + list_type_depth`, and the
/// zero-token counters are reset only at a value/closer (where the active chain
/// reduces), **never** at an opener (where the outer chain stays frozen-open and
/// must keep counting). The delimiter caps [`MAX_NESTING_DEPTH`] (64) and
/// [`MAX_LIST_NESTING_DEPTH`] (32) remain as tighter, more precise sub-bounds.
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
    // word) or a closer (`)`/`}`/`]`/`END`/`>`), where the active unary/`NOT`
    // chain reduces; an OPENER (`(`/`{`/`[`/`CASE`/`LIST`/sign/`NOT`) never resets
    // the other counters, so a chain frozen open across a delimiter keeps being
    // counted (otherwise the guard would admit the product of the per-counter
    // caps — see `MAX_RECURSION_DEPTH`). Whitespace/comment arms `continue`
    // without touching any counter.
    let mut sign_run = 0_u32;
    let mut not_run = 0_u32;
    // `CASE` body nesting: +1 per `CASE`, -1 per `END`.
    let mut case_depth = 0_u32;
    // `LIST<…>` type-name nesting: +1 per `LIST` keyword, -1 per `>`. The depth
    // only rises inside a `LIST<…>` whose contents are types (no comparison
    // operators), so a `>` belonging to a comparison can only appear while the
    // depth is already 0, where the saturating decrement is a no-op.
    let mut list_type_depth = 0_u32;

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
                // An opener does NOT reset the sign / `NOT` runs: the delimiter
                // expression is the operand of any enclosing unary chain, which
                // stays open and must keep counting toward the combined cap.
                depth += 1;
                if depth > MAX_NESTING_DEPTH {
                    return Err(ParserError::NestingLimitExceeded {
                        limit: MAX_NESTING_DEPTH,
                        span: point_span(index),
                    });
                }
                if exceeds_recursion_budget(depth, sign_run, not_run, case_depth, list_type_depth) {
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
                if exceeds_recursion_budget(depth, sign_run, not_run, case_depth, list_type_depth) {
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
                if exceeds_recursion_budget(depth, sign_run, not_run, case_depth, list_type_depth) {
                    return Err(ParserError::ComplexityLimitExceeded {
                        limit: MAX_RECURSION_DEPTH,
                        span: point_span(index),
                    });
                }
            }
            // `>` closes one `LIST<…>` type-name nesting level. It is also a
            // primary/separator, so it terminates a leading-sign / `NOT` chain.
            // Outside a `LIST<…>` (depth 0) — e.g. a `comp_op` `>` — the
            // saturating decrement is a no-op, so comparison chains never
            // underflow or false-trip the cap.
            b'>' => {
                sign_run = 0;
                not_run = 0;
                list_type_depth = list_type_depth.saturating_sub(1);
            }
            // Whitespace is transparent to all counters (pest skips it).
            b' ' | b'\t' | b'\r' | b'\n' => {}
            // An ASCII identifier-start byte begins a whole word. Recognize it
            // case-insensitively: `NOT`/`CASE`/`LIST` are openers (they add a
            // recursion frame, checked against the combined budget, and do NOT
            // reset the other open chains); `END` is a closer; any other word is
            // a value/primary that reduces the active unary / `NOT` chain.
            byte if is_word_start(byte) => {
                let (word_kind, next_index) = recognize_word(bytes, index);
                index = next_index;
                match word_kind {
                    WordKind::Not => {
                        not_run += 1;
                        if exceeds_recursion_budget(
                            depth,
                            sign_run,
                            not_run,
                            case_depth,
                            list_type_depth,
                        ) {
                            return Err(ParserError::ComplexityLimitExceeded {
                                limit: MAX_RECURSION_DEPTH,
                                span: point_span(index),
                            });
                        }
                    }
                    WordKind::Case => {
                        case_depth += 1;
                        if exceeds_recursion_budget(
                            depth,
                            sign_run,
                            not_run,
                            case_depth,
                            list_type_depth,
                        ) {
                            return Err(ParserError::ComplexityLimitExceeded {
                                limit: MAX_RECURSION_DEPTH,
                                span: point_span(index),
                            });
                        }
                    }
                    WordKind::End => {
                        case_depth = case_depth.saturating_sub(1);
                        sign_run = 0;
                        not_run = 0;
                    }
                    WordKind::List => {
                        list_type_depth += 1;
                        if exceeds_recursion_budget(
                            depth,
                            sign_run,
                            not_run,
                            case_depth,
                            list_type_depth,
                        ) {
                            return Err(ParserError::ComplexityLimitExceeded {
                                limit: MAX_RECURSION_DEPTH,
                                span: point_span(index),
                            });
                        }
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
            // primary or separator: it reduces the active sign / `NOT` chain.
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
/// (`sign_run`) plus the `NOT` chain (`not_run`) plus open `CASE` bodies
/// (`case_depth`) plus open `LIST<…>` levels (`list_type_depth`). Bounding the
/// SUM — not each counter independently — is what prevents the product-depth
/// vector (e.g. nested `(` each carrying a long sign chain). `saturating_add`
/// guards against overflow even though each addend is checked at +1 increments.
fn exceeds_recursion_budget(
    depth: u32,
    sign_run: u32,
    not_run: u32,
    case_depth: u32,
    list_type_depth: u32,
) -> bool {
    depth
        .saturating_add(sign_run)
        .saturating_add(not_run)
        .saturating_add(case_depth)
        .saturating_add(list_type_depth)
        > MAX_RECURSION_DEPTH
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
    /// `LIST` (case-insensitive, whole word) — opens a `LIST<…>` `type_name`
    /// nesting level (the only `<`-delimited recursive type constructor). A
    /// matching `>` closes it. `COLLECT_LIST` and other words containing `LIST`
    /// are distinct whole words and do not match.
    List,
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
    } else if word.eq_ignore_ascii_case(b"LIST") {
        WordKind::List
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
