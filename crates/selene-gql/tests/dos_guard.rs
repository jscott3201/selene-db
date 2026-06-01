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

// ---------------------------------------------------------------------------
// BRIEF 01b positive-boundary cases: the zero-delimiter recursion guard
// (`MAX_RECURSION_DEPTH = 256`, drivers: unary sign / `NOT` / `CASE`) must not
// regress any legitimate query, and the `--`-as-comment removal must not break
// directed edges or any other construct.
// ---------------------------------------------------------------------------

#[test]
fn single_digit_unary_sign_nesting_parses() {
    // A handful of leading signs is legitimate and far below the cap.
    parse("RETURN ---1").expect("triple-negated integer parses");
    parse("RETURN +-+2").expect("mixed-sign integer parses");
    parse("RETURN -1.5").expect("negative float parses");
    parse("RETURN +0.25").expect("explicit-positive float parses");
}

#[test]
fn negative_numbers_and_subtraction_parse() {
    // Binary subtraction (`a - b - c`) is a chain of `sign_op`-looking bytes but
    // never a *leading* unary run, so it stays at run length ~1-2 and parses.
    parse("RETURN 10 - 3 - 2").expect("left-associative subtraction parses");
    parse("RETURN -5 + 3").expect("negative literal plus literal parses");
}

#[test]
fn double_negation_not_not_parses() {
    // `NOT NOT x` is legitimate double-negation (run length 2, well under cap).
    let item = parse("RETURN NOT NOT true").expect("double negation parses");
    assert!(matches!(item, selene_gql::Statement::Query(_)));
}

#[test]
fn real_case_when_then_else_end_parses() {
    parse("RETURN CASE WHEN true THEN 1 ELSE 0 END").expect("searched CASE parses");
    parse("RETURN CASE 1 WHEN 1 THEN 'a' WHEN 2 THEN 'b' ELSE 'c' END")
        .expect("simple CASE parses");
    // Modest nesting (depth 3) is legitimate and below the cap.
    parse("RETURN CASE WHEN true THEN CASE WHEN false THEN 1 ELSE 2 END ELSE 3 END")
        .expect("nested CASE at depth 2 parses");
}

#[test]
fn directed_edges_parse_after_dash_comment_removal() {
    // The `--`-as-line-comment arm was removed (the grammar's only line comment
    // is `//`). A directed edge `->` / `<-` / `-[r]->` contains `-`/`->`; the
    // guard must NOT swallow the rest of the line as a comment (the old `--` arm
    // did, after the first `--` on a line), and the dashes in the edge syntax
    // must not be misread as a unary run. (The grammar's abbreviated directed
    // edge is `->`, not the Cypher-style `-->`; `-[r]->` is the bracketed form.)
    parse("MATCH (a)->(b) RETURN b").expect("abbreviated right-directed edge parses");
    parse("MATCH (a)<-(b) RETURN b").expect("abbreviated left-directed edge parses");
    parse("MATCH (a)-[r]->(b) RETURN r").expect("bracketed directed edge parses");
    parse("MATCH (a)-[r:KNOWS]->(b) RETURN r").expect("labeled directed edge parses");
    parse("MATCH (a)<-[r:KNOWS]-(b) RETURN r").expect("left-bracketed directed edge parses");
}

#[test]
fn numeric_range_quantifier_parses() {
    parse("MATCH (a)-[:E*1..3]->(b) RETURN b").expect("bounded variable-length path parses");
}

#[test]
fn dash_dash_after_value_is_not_a_comment() {
    // `RETURN 1 -- 2` is `1` minus unary-minus `2` (a double sign), NOT a SQL
    // line comment. With the `--` arm removed the guard no longer hides the
    // `2`; pest parses it as `1 - (-2)`.
    parse("RETURN 1 -- 2").expect("`--` is two signs, not a line comment");
}

#[test]
fn slash_slash_line_comment_still_works() {
    // The real line comment (`//`) is untouched.
    parse("RETURN 1 // trailing comment").expect("// line comment still parses");
    parse("RETURN 1 /* block */ AS x").expect("/* */ block comment still parses");
}
