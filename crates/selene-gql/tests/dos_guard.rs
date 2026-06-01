//! Parser bracket-depth / nesting DoS guard coverage (#218).

use selene_gql::{ParserError, parse};

const NESTING_LIMIT: usize = 64;
/// Mirror of `guard::MAX_LIST_NESTING_DEPTH` (the public crate does not
/// re-export it). A `[` run deeper than this is rejected by the complexity cap.
const LIST_NESTING_CAP: usize = 32;

#[test]
fn rejects_excessive_bracket_run_before_pest_parse() {
    // A deeply nested run of `[` openers is rejected by the pre-pest byte-scan
    // guard. The `[`-specific complexity cap (32 nested list brackets) is the
    // tighter of the two byte-scan caps and fires first: a `[` run deep enough
    // to approach the 64-delimiter general nesting cap has already exceeded the
    // tighter `[`-depth cap, so it is reported as the (more precise)
    // ComplexityLimitExceeded rather than NestingLimitExceeded. Both map to
    // GQLSTATUS 5GQL1 PROGRAM_LIMIT_EXCEEDED.
    let source = format!(
        "LET x = {}0{} RETURN x",
        "[".repeat(NESTING_LIMIT + 1),
        "]".repeat(NESTING_LIMIT + 1)
    );
    let error = parse(&source).expect_err("over-complex parse rejects");
    assert!(
        matches!(error, ParserError::ComplexityLimitExceeded { .. }),
        "expected ComplexityLimitExceeded, got {error:?}"
    );
}

#[test]
fn rejects_excessive_type_name_list_nesting() {
    let depth = NESTING_LIMIT + 1;
    let source = format!(
        "CREATE NODE TYPE :Deep (v :: {}INTEGER{})",
        "LIST<".repeat(depth),
        ">".repeat(depth)
    );
    let error = parse(&source).expect_err("over-nested type name rejects");
    assert!(matches!(
        error,
        ParserError::NestingLimitExceeded { limit: 64, .. }
    ));
}

#[test]
fn rejects_excessive_record_type_nesting() {
    // `RECORD{ ... }` nesting opens one `{` per level. `{` is not a demonstrated
    // super-linear backtracking vector — only `[` carries the tighter dedicated
    // complexity cap — so a `{` run past the general all-bracket nesting cap is
    // reported as a NestingLimitExceeded violation. Both map to GQLSTATUS 5GQL1
    // PROGRAM_LIMIT_EXCEEDED.
    let depth = NESTING_LIMIT + 1;
    let source = format!(
        "CREATE NODE TYPE :DeepRecord (v :: {}INTEGER{})",
        "RECORD{f :: ".repeat(depth),
        "}".repeat(depth)
    );
    let error = parse(&source).expect_err("over-deep record type name rejects");
    assert!(
        matches!(error, ParserError::NestingLimitExceeded { .. }),
        "expected NestingLimitExceeded, got {error:?}"
    );
}

#[test]
fn nesting_guard_ignores_quoted_and_commented_delimiters() {
    let noisy_string = "[".repeat(NESTING_LIMIT + 32);
    let noisy_comment = "(".repeat(NESTING_LIMIT + 32);
    let source = format!("// {noisy_comment}\nRETURN '{noisy_string}'");
    parse(&source).expect("delimiters inside comments and strings are ignored by the guard");
}

#[test]
fn dangling_backslash_quote_does_not_hide_hostile_brackets() {
    // Regression for the guard/pest desync (Codex review on PR #218): for
    // `RETURN '\'` with no later quote, pest treats the `\` as a dangling
    // escape, closes the string at that `'`, then parses the following `[` run.
    // A blanket "`\` escapes the next byte" pre-scan would instead skip to EOF,
    // hiding the brackets and leaving the parse-time DoS reachable behind a
    // 3-byte prefix. The guard must close the string at the same `'` and still
    // count the brackets.
    let hostile = format!("RETURN '\\', {}0", "[".repeat(LIST_NESTING_CAP + 8));
    let error =
        parse(&hostile).expect_err("dangling backslash-quote must not hide the bracket run");
    assert!(
        matches!(error, ParserError::ComplexityLimitExceeded { .. }),
        "expected ComplexityLimitExceeded, got {error:?}"
    );
}

#[test]
fn escaped_quote_with_later_quote_keeps_brackets_inside_the_string() {
    // The dual: when a `\'` has a LATER quote it is a genuine escaped quote
    // (pest `escaped_quote`), so the string continues and the brackets inside
    // it are content, not code. A deep `[` run inside such a string must NOT be
    // rejected — the guard must mirror pest here too and not over-count.
    let in_string = format!("RETURN 'a\\'b {}'", "[".repeat(LIST_NESTING_CAP + 8));
    parse(&in_string).expect("brackets inside an escaped-quote string are not counted");
}
