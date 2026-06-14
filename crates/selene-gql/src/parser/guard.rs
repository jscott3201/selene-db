//! Cheap parser input guards that run before pest recursive descent.

use crate::{SourceSpan, error::ParserError};

/// Maximum syntactic nesting depth admitted by the parser.
///
/// This bounds pest recursion on hostile malformed expressions while leaving
/// ordinary query, list, record, and subquery nesting comfortably below the cap.
///
/// **Load-bearing safety invariant — do not raise without re-checking the stack
/// budget.** This cap also bounds *subquery* nesting (`EXISTS`/`COUNT`/`VALUE`/
/// `CALL { … }` each open a counted `{`). The post-build expression-depth guard
/// (`super::depth`) resets its depth counter at each subquery boundary, so the
/// recursive Flagger walk's worst-case native-stack depth is the *product*
/// `MAX_NESTING_DEPTH × depth::MAX_EXPR_DEPTH` (≈ 64 × 256 ≈ 16 k frames) — a
/// deeply nested subquery whose every level carries a max-depth expression. That
/// fits only because `parse` runs the whole descent + build + Flagger on the
/// 32 MB `PARSE_STACK_SEGMENT` (`parser/mod.rs`). Raising this cap
/// (or `MAX_EXPR_DEPTH`, or shrinking the segment) must keep
/// `MAX_NESTING_DEPTH × MAX_EXPR_DEPTH × per_frame_bytes` safely under the
/// segment, or the non-unwindable Flagger overflow re-opens with no guard to
/// catch it. See `tests/parser_expr_depth.rs::nested_subqueries_with_deep_folds_do_not_crash`.
pub(crate) const MAX_NESTING_DEPTH: u32 = 64;

/// Maximum simultaneously-open `[` list nesting depth admitted in a single
/// statement.
///
/// pest is not packrat-memoized, so a run of unclosed list-literal openers can
/// still drive expensive nested expression attempts before the parser can
/// reject the input. Parsing precedes execution, so an execution deadline cannot
/// interrupt the blow-up; the only safe place to stop it is before recursive
/// descent begins.
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
/// - `unary = { sign_op ~ unary | postfix }` (`grammar.pest:380`) — a *run* of
///   leading unary `+`/`-` signs.
/// - `not_expr = { not_kw ~ not_expr | is_expr }` (`grammar.pest:324`) — a *run*
///   of `NOT` keywords.
/// - `case_expr` ↔ `expr` (`grammar.pest:447`) — *nested* `CASE … END`
///   expressions: each level re-enters the full precedence cascade for its
///   `WHEN`/`THEN`/`ELSE` operand exprs. `CASE` is keyword-delimited, and a byte
///   scan cannot soundly recognize its `END` *closer* (see below), so this is
///   tracked by a **monotone** `case_depth` — a sound over-approximation of
///   open-`CASE` pressure that never decrements — rather than a balanced counter.
///
/// `NOT`/`CASE`/`END` are reserved keywords (`grammar.pest` `keyword` set), so
/// they cannot be *bare* identifiers — but `prop_ident` (`grammar.pest:602`) has
/// no keyword guard, so they appear as identifiers in property names (`n.END`),
/// map/record keys (`{END: …}`), aliases (`AS END`), `YIELD` columns
/// (`YIELD a, END`), and parameters (`$END`). This is why `case_depth` is
/// **monotone**, with an asymmetry between the opener and the closer:
///
/// - A `CASE` **opener** is recognized soundly. It is counted only when *not* in
///   an identifier position (`.`/`$` lookbehind, an `AS`/`YIELD` predecessor, or
///   a `:` follower — see [`validate`]), and the grammar never places a real
///   `case_expr` in any of those slots, so skipping an identifier-`CASE` can
///   never *under*-count a real frame.
/// - The `END` **closer** cannot be told apart from an identifier `END` in every
///   position — a comma-tail `YIELD` column (`CALL p() YIELD a, END`) is a bare
///   `prop_ident` that a lookbehind/lookahead cannot distinguish from the
///   keyword. So the guard **never decrements** on `END`. Trusting `END` as a
///   closer would let a phantom identifier-`END` cancel a real open `CASE` and
///   reopen the overflow (a confirmed bypass).
///
/// The cost of monotonicity is a *documented, conformant* limit: a single
/// statement's combined `CASE`-count-plus-nesting-pressure must stay ≤
/// [`MAX_RECURSION_DEPTH`], so a pathological statement with hundreds of (even
/// non-nested) `CASE` expressions is rejected with the program-limit GQLSTATUS.
/// Realistic queries (a handful of `CASE`s, shallow nesting) are unaffected — the
/// deepest legitimate nesting anywhere in the workspace is 3.
///
/// (The `type_name` `LIST<…>` recursion is *also* zero-delimiter, but `LIST` is
/// **not** reserved — it is a legal bare variable and `LIST < x` is a comparison
/// — so a byte scan cannot soundly distinguish the type opener from a variable.
/// `LIST<…>` is instead bounded at AST-build time by `build_type_name_with_depth`
/// (`builders::expr`, depth 64); only the pre-build pest-descent tail of a very
/// large input remains, absorbed by the `stacker` parse backstop in
/// `parser/mod.rs`.)
///
/// A long enough zero-delimiter run/nesting overflows the native stack inside
/// pest. A Rust stack overflow is **non-unwindable** (`catch_unwind` cannot trap
/// it), so a small hostile string would hard-kill the host process embedding
/// selene-db. This pre-pest cap rejects such inputs deterministically before
/// recursive descent begins, mapped to GQLSTATUS `5GQL1`.
///
/// **This is a single COMBINED ceiling on total recursion pressure**, not a
/// per-counter cap. The native stack depth at the deepest point is the *sum* of
/// every simultaneously-open recursion frame — open delimiters (`depth`) *plus*
/// the current unary-sign chain *plus* the `NOT` chain *plus* the open `CASE`
/// frames — because they nest as operands of one another (`(` `-` `CASE WHEN (` …
/// each frame stays open until its operand reduces). Capping each counter
/// independently would admit their *product* (e.g. 64 nested `(` each carrying a
/// 255-deep sign chain ≈ 16 k frames) while every individual counter stayed under
/// its cap — a real stack-overflow vector. So the guard bounds the running SUM
/// `depth + sign_run + not_run + case_depth`, maintained as a sound upper bound
/// on the live native-stack depth at every position
/// (`depth + sign_run + not_run + case_depth ≥ true_open_frames`), so the cap is
/// detected at or before the deepest point. The counters differ in how they
/// reduce:
///
/// - The *run* counters (`sign_run`, `not_run`) reset at a value/closer (where
///   the active chain reduces) but never at an opener.
/// - `depth` is a balanced delimiter counter (decrements at `)`/`}`/`]`).
/// - `case_depth` is **monotone** (never decrements — see above). A real `CASE`
///   opener folds the wrapping sign/`NOT` run into it
///   (`case_depth += 1 + sign_run + not_run`) before resetting those runs, so the
///   run frames that stay open across the `CASE` operands are not lost. This is
///   what bounds `NOT … NOT CASE … THEN NOT … NOT CASE …`, whose true descent
///   depth is `levels × (run + 1)` — folding keeps the sum ≥ that depth.
///
/// The delimiter caps [`MAX_NESTING_DEPTH`] (64) and [`MAX_LIST_NESTING_DEPTH`]
/// (32) remain as tighter, more precise sub-bounds.
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
    let last_double_quote = bytes.iter().rposition(|byte| *byte == b'"');
    let last_backtick = bytes.iter().rposition(|byte| *byte == b'`');
    let mut index = 0;
    let mut depth = 0_u32;
    let mut list_depth = 0_u32;
    // Recursion-pressure counters (see `MAX_RECURSION_DEPTH`). Their SUM with
    // `depth` is the bounded quantity: it tracks the native stack depth at the
    // current position. pest treats comments as whitespace, so a comment between
    // two signs does NOT break the unary run (`- /* c */ -` is a 2-deep `unary`).
    // The *run* counters (`sign_run`, `not_run`) reset at a value (a primary —
    // string/backtick/number/other word) or a closer; the *balanced* counters
    // (`depth`, `case_depth`) decrement only at their matching closer (delimiter
    // closer, keyword `END`). An OPENER never resets a run, so a chain frozen open
    // across a delimiter keeps counting (otherwise the guard would admit the
    // product of the per-counter caps — see `MAX_RECURSION_DEPTH`).
    // Whitespace/comment arms `continue` without touching any counter OR the
    // `prev_*` lookbehind state, so adjacency survives an intervening comment.
    let mut sign_run = 0_u32;
    let mut not_run = 0_u32;
    // Monotone nested-`CASE` pressure (see `MAX_RECURSION_DEPTH`). A real keyword
    // `CASE` opener adds 1 plus the sign/`NOT` run that wraps it; it NEVER
    // decrements (a byte scan cannot soundly tell a keyword `END` closer from an
    // identifier `END`, e.g. a `YIELD` column). Counted only when the `CASE` word
    // is not in an identifier position — see the word arm.
    let mut case_depth = 0_u32;
    // Lookbehind for the word classifier. `prev_sig_byte` is the last
    // *significant* (non-whitespace, non-comment) byte; only `.`/`$` matter (a
    // word right after them is a `prop_ident`/`param_ref` identifier, never the
    // keyword). `prev_word` is the last significant *keyword word* that admits an
    // identifier after it (`AS <alias>`, `YIELD <item>`); a word right after one
    // of those is an identifier. Both are left UNCHANGED by whitespace/comment.
    let mut prev_sig_byte: Option<u8> = None;
    let mut prev_word = PrevWord::Other;

    while index < bytes.len() {
        match bytes[index] {
            // Quoted string/identifier spans are primaries for guard purposes:
            // they reset the unary and `NOT` runs (a primary terminates a
            // leading-sign / `NOT` chain), then the scan resumes after the
            // closing quote.
            b'@' if next_is(bytes, index, b'\'') => {
                sign_run = 0;
                not_run = 0;
                prev_word = PrevWord::Other;
                prev_sig_byte = Some(b'\'');
                index = skip_no_escape_quoted(bytes, index + 2, b'\'');
            }
            b'@' if next_is(bytes, index, b'"') => {
                sign_run = 0;
                not_run = 0;
                prev_word = PrevWord::Other;
                prev_sig_byte = Some(b'"');
                index = skip_no_escape_quoted(bytes, index + 2, b'"');
            }
            b'@' if next_is(bytes, index, b'`') => {
                sign_run = 0;
                not_run = 0;
                prev_word = PrevWord::Other;
                prev_sig_byte = Some(b'`');
                index = skip_no_escape_quoted(bytes, index + 2, b'`');
            }
            b'\'' => {
                sign_run = 0;
                not_run = 0;
                prev_word = PrevWord::Other;
                prev_sig_byte = Some(b'\'');
                index = skip_single_quoted(bytes, index + 1, last_single_quote);
            }
            b'"' => {
                sign_run = 0;
                not_run = 0;
                prev_word = PrevWord::Other;
                prev_sig_byte = Some(b'"');
                index = skip_double_quoted(bytes, index + 1, last_double_quote);
            }
            b'`' => {
                sign_run = 0;
                not_run = 0;
                prev_word = PrevWord::Other;
                prev_sig_byte = Some(b'`');
                index = skip_backtick_quoted(bytes, index + 1, last_backtick);
            }
            // Comments are whitespace to pest, so they do NOT reset any run or
            // the `prev_*` lookbehind: `- // c\n -` is still a 2-deep unary chain,
            // and `n ./* c */ END` still sees `.` before `END`. Skip the span and
            // continue without touching state. `--` is two unary minus `sign_op`
            // (`grammar.pest:642` defines `//` as the only line comment), so it is
            // handled by the `b'-'` arm, NOT skipped here.
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
                prev_word = PrevWord::Other;
                prev_sig_byte = Some(bytes[index]);
                if depth > MAX_NESTING_DEPTH {
                    return Err(ParserError::NestingLimitExceeded {
                        limit: MAX_NESTING_DEPTH,
                        span: point_span(index),
                    });
                }
                if exceeds_recursion_budget(depth, sign_run, not_run, case_depth) {
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
                prev_word = PrevWord::Other;
                prev_sig_byte = Some(b'[');
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
                if exceeds_recursion_budget(depth, sign_run, not_run, case_depth) {
                    return Err(ParserError::ComplexityLimitExceeded {
                        limit: MAX_RECURSION_DEPTH,
                        span: point_span(index),
                    });
                }
            }
            // A closer reduces the delimiter expression, so the unary / `NOT`
            // chain that wrapped it (if any) is now complete: reset both runs.
            // `case_depth` is NOT touched — a `)` closes a paren, not a `CASE`.
            b')' | b'}' => {
                depth = depth.saturating_sub(1);
                sign_run = 0;
                not_run = 0;
                prev_word = PrevWord::Other;
                prev_sig_byte = Some(bytes[index]);
            }
            b']' => {
                depth = depth.saturating_sub(1);
                list_depth = list_depth.saturating_sub(1);
                sign_run = 0;
                not_run = 0;
                prev_word = PrevWord::Other;
                prev_sig_byte = Some(b']');
            }
            // A leading unary `+`/`-` opens one more `unary` frame. `--` is two
            // signs (not a comment); `->`/`-[r]-` and binary `a-b-c` never run
            // beyond ~2-3 before a value resets the run. It is an opener, so it
            // does not reset `not_run`; the combined budget bounds the depth.
            b'+' | b'-' => {
                sign_run += 1;
                prev_word = PrevWord::Other;
                prev_sig_byte = Some(bytes[index]);
                if exceeds_recursion_budget(depth, sign_run, not_run, case_depth) {
                    return Err(ParserError::ComplexityLimitExceeded {
                        limit: MAX_RECURSION_DEPTH,
                        span: point_span(index),
                    });
                }
            }
            // Whitespace is transparent to all counters and lookbehind (pest
            // skips it), so it leaves every counter and `prev_*` UNCHANGED.
            b' ' | b'\t' | b'\r' | b'\n' => {}
            // An identifier-start byte begins a whole word. UTF-8 lead/continuation
            // bytes (>= 0x80) route here too so a Unicode identifier (`éCASE`) is
            // consumed whole and never mis-segments its ASCII tail as a keyword.
            byte if is_word_byte_start(byte) => {
                let word_end = scan_word_chars(source, index);
                if word_end == index {
                    // A `>= 0x80` lead byte that is not an identifier char
                    // (Unicode punctuation / space): a separator/primary. Advance
                    // one whole char to keep `index` on a char boundary.
                    sign_run = 0;
                    not_run = 0;
                    prev_word = PrevWord::Other;
                    prev_sig_byte = Some(byte);
                    index += char_len_at(source, index);
                    continue;
                }
                // A keyword in any of these contexts is a `prop_ident`/`param_ref`
                // identifier, not the keyword. The lookbehind (`.`/`$`, AS/YIELD)
                // and `:`/`::` lookahead skip whitespace+comments, mirroring pest's
                // non-atomic `~`.
                let in_ident_pos = matches!(prev_sig_byte, Some(b'.') | Some(b'$'))
                    || matches!(prev_word, PrevWord::As | PrevWord::Yield)
                    || next_sig_is_colon(bytes, word_end);
                let class = classify_word(&source[index..word_end]);
                match class {
                    WordClass::Not if !in_ident_pos => {
                        not_run += 1;
                        if exceeds_recursion_budget(depth, sign_run, not_run, case_depth) {
                            return Err(ParserError::ComplexityLimitExceeded {
                                limit: MAX_RECURSION_DEPTH,
                                span: point_span(index),
                            });
                        }
                    }
                    WordClass::Case if !in_ident_pos => {
                        // A real `CASE` opener. `case_depth` is a *monotone* sound
                        // over-approximation of open-`CASE` stack pressure: it never
                        // decrements (a byte scan cannot reliably tell a keyword
                        // `END` closer from an identifier `END` — e.g. a `YIELD`
                        // column — so trusting `END` as a closer is unsound). Fold
                        // the sign/`NOT` run that *wraps* this `CASE` (those frames
                        // stay open across its operands) into `case_depth`, then
                        // reset the runs so they are not double-counted. See
                        // `MAX_RECURSION_DEPTH`.
                        case_depth = case_depth
                            .saturating_add(1)
                            .saturating_add(sign_run)
                            .saturating_add(not_run);
                        sign_run = 0;
                        not_run = 0;
                        if exceeds_recursion_budget(depth, sign_run, not_run, case_depth) {
                            return Err(ParserError::ComplexityLimitExceeded {
                                limit: MAX_RECURSION_DEPTH,
                                span: point_span(index),
                            });
                        }
                    }
                    // Every other word — a value/primary, an identifier-positioned
                    // `CASE`, or any `END`/`NOT`/`AS`/`YIELD` (`AS`/`YIELD` act via
                    // the `prev_word` lookbehind below, not here) — terminates the
                    // active sign/`NOT` run but never decrements the monotone
                    // `case_depth`. In particular a keyword `END` is treated exactly
                    // like an ordinary word: it is NOT a trusted closer.
                    _ => {
                        sign_run = 0;
                        not_run = 0;
                    }
                }
                // Record the lookback for the NEXT word: only a genuine (keyword,
                // not identifier-positioned) AS/YIELD makes the following word an
                // identifier.
                prev_word = if in_ident_pos {
                    PrevWord::Other
                } else {
                    match class {
                        WordClass::As => PrevWord::As,
                        WordClass::Yield => PrevWord::Yield,
                        _ => PrevWord::Other,
                    }
                };
                // The word itself is now the predecessor — a non-`.`/`$` byte.
                prev_sig_byte = Some(b'w');
                index = word_end;
                continue;
            }
            // Any other byte (operators `.`, `,`, `*`, `<`, `>`, `:`, `$`, digits,
            // etc.) is a primary or separator: it reduces the active sign / `NOT`
            // chain. `prev_sig_byte` records it so a following word can see a
            // leading `.`/`$`.
            other => {
                sign_run = 0;
                not_run = 0;
                prev_word = PrevWord::Other;
                prev_sig_byte = Some(other);
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
/// (`sign_run`) plus the `NOT` chain (`not_run`) plus the open nested-`CASE`
/// frames (`case_depth`). Bounding the SUM — not each counter independently — is
/// what prevents the product-depth vector (e.g. nested `(` each carrying a long
/// sign chain, or nested `CASE` each carrying a sign run). `saturating_add`
/// guards against overflow even though each addend is checked at +1 increments.
fn exceeds_recursion_budget(depth: u32, sign_run: u32, not_run: u32, case_depth: u32) -> bool {
    depth
        .saturating_add(sign_run)
        .saturating_add(not_run)
        .saturating_add(case_depth)
        > MAX_RECURSION_DEPTH
}

fn next_is(bytes: &[u8], index: usize, expected: u8) -> bool {
    bytes.get(index + 1).is_some_and(|value| *value == expected)
}

/// Keyword classification of a complete identifier word.
///
/// Only the keywords the guard reacts to are distinguished; everything else
/// (including all non-keyword identifiers) is [`WordClass::Other`].
enum WordClass {
    /// `NOT` — a unary-run opener.
    Not,
    /// `CASE` — opens a (monotone) nested-`CASE` frame.
    Case,
    /// `END` — the nested-`CASE` closer keyword. **Deliberately treated as an
    /// ordinary word** (no `case_depth` decrement): a byte scan cannot reliably
    /// tell a keyword `END` from an identifier `END` (a `YIELD` column, property
    /// name, or map key), so `case_depth` is a monotone over-approximation that
    /// never trusts `END` as a closer. See [`MAX_RECURSION_DEPTH`].
    End,
    /// `AS` — the following `prop_ident` is an alias identifier.
    As,
    /// `YIELD` — the following `prop_ident` is a yield-column identifier.
    Yield,
    /// Any other word.
    Other,
}

/// The last significant keyword word that makes the *next* word an identifier.
enum PrevWord {
    /// Last significant word was the `AS` keyword.
    As,
    /// Last significant word was the `YIELD` keyword.
    Yield,
    /// Anything else.
    Other,
}

/// A byte that may begin a GQL identifier word: ASCII `[A-Za-z_]` or any UTF-8
/// lead byte (`>= 0x80`).
///
/// `grammar.pest`'s `ident`/`prop_ident` admit Unicode `LETTER`, so a Unicode
/// identifier like `éCASE` must be consumed as one word — otherwise an ASCII-only
/// scan would split it and mis-read the trailing `CASE` as the keyword. Routing
/// every `>= 0x80` lead byte into the word arm (which then scans whole `char`s)
/// makes the segmentation match pest.
fn is_word_byte_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

/// Byte length of the `char` beginning at `index` (≥ 1; defaults to 1 at EOF).
fn char_len_at(source: &str, index: usize) -> usize {
    source[index..].chars().next().map_or(1, char::len_utf8)
}

/// Consume one whole Unicode identifier word starting at `start` and return the
/// byte index just past it (`== start` if the char at `start` is not an
/// identifier char).
///
/// Word boundaries match the grammar's `!(LETTER | NUMBER | "_")` keyword guard
/// over Unicode `char`s, so `NOTE`/`éCASE` are single words and never match a
/// keyword. Operates on `char`s (not bytes) so the returned index always lands on
/// a UTF-8 char boundary.
fn scan_word_chars(source: &str, start: usize) -> usize {
    let mut end = start;
    for (offset, ch) in source[start..].char_indices() {
        if ch.is_alphanumeric() || ch == '_' {
            end = start + offset + ch.len_utf8();
        } else {
            break;
        }
    }
    end
}

/// Classify a complete identifier word (ASCII-case-insensitively, matching the
/// `^"…"` case-insensitive keyword rules).
fn classify_word(word: &str) -> WordClass {
    if word.eq_ignore_ascii_case("NOT") {
        WordClass::Not
    } else if word.eq_ignore_ascii_case("CASE") {
        WordClass::Case
    } else if word.eq_ignore_ascii_case("END") {
        WordClass::End
    } else if word.eq_ignore_ascii_case("AS") {
        WordClass::As
    } else if word.eq_ignore_ascii_case("YIELD") {
        WordClass::Yield
    } else {
        WordClass::Other
    }
}

/// Whether the first significant (non-whitespace, non-comment) byte at or after
/// `from` is a `:` (covering both the `:` of a record/property/map key and the
/// `::` of a typed record field — `::` starts with `:`).
///
/// Skips whitespace and both comment forms because pest's non-atomic `~` allows
/// them between a `prop_ident` key and its `:` (`{ CASE /* */ : 1 }`). A word
/// followed by `:`/`::` is a key identifier, never a real `CASE` opener (no
/// grammar production places `:` after a `case_expr`). (`END` is never counted
/// regardless of position, so this lookahead is only load-bearing for `CASE`.)
fn next_sig_is_colon(bytes: &[u8], from: usize) -> bool {
    let mut index = from;
    while index < bytes.len() {
        match bytes[index] {
            b' ' | b'\t' | b'\r' | b'\n' => index += 1,
            b'/' if next_is(bytes, index, b'/') => index = skip_line_comment(bytes, index + 2),
            b'/' if next_is(bytes, index, b'*') => index = skip_block_comment(bytes, index + 2) + 1,
            b':' => return true,
            _ => return false,
        }
    }
    false
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

fn skip_double_quoted(bytes: &[u8], mut index: usize, last_quote: Option<usize>) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if bytes.get(index + 1) == Some(&b'"') && Some(index + 1) == last_quote => {
                return index + 1;
            }
            b'\\' => index += 2,
            b'"' if next_is(bytes, index, b'"') => index += 2,
            b'"' => return index,
            _ => index += 1,
        }
    }
    bytes.len()
}

fn skip_no_escape_quoted(bytes: &[u8], mut index: usize, delimiter: u8) -> usize {
    while index < bytes.len() {
        if bytes[index] == delimiter {
            return index;
        }
        index += 1;
    }
    bytes.len()
}

fn skip_backtick_quoted(bytes: &[u8], mut index: usize, last_backtick: Option<usize>) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if bytes.get(index + 1) == Some(&b'`') && Some(index + 1) == last_backtick => {
                return index + 1;
            }
            b'\\' => index += 2,
            b'`' if next_is(bytes, index, b'`') => index += 2,
            b'`' => return index,
            _ => index += 1,
        }
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
