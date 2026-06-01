//! Pre-pest nested-`CASE … END` depth guard coverage (node 818, Family A).
//!
//! `case_expr ↔ expr` is a zero-delimiter pest recursion: each nested
//! `CASE WHEN … THEN … END` re-enters the full precedence cascade for its
//! operand expressions, overflowing the native stack at ~2264 nesting (a ~53 KB
//! input) — a non-unwindable crash. The pre-pest byte-scan guard tracks a
//! *balanced* `case_depth` (keyword `CASE` opens, keyword `END` closes) folded
//! into the combined recursion ceiling (256), rejecting hostile nesting as
//! `ComplexityLimitExceeded` (GQLSTATUS 5GQL1) before pest runs.
//!
//! The crux is SOUND keyword recognition: `CASE`/`END` are reserved but appear
//! as identifiers (via `prop_ident`) in property names, map/record keys,
//! aliases, `YIELD` items, and parameters. The guard must NOT count those — a
//! prior attempt (PR #224) let `{END: 1}` decrement `case_depth` and re-opened
//! the crash. These tests assert (a) hostile nesting rejects, (b) the `{END:1}`
//! bypass is closed, and (c) every identifier position still parses.

use selene_gql::{GqlStatus, ParserError, parse};

const RECURSION_DEPTH_CAP: usize = 256;

fn assert_complexity_rejected(source: &str, label: &str) {
    let error = parse(source).expect_err(label);
    assert!(
        matches!(error, ParserError::ComplexityLimitExceeded { limit, .. } if limit as usize == RECURSION_DEPTH_CAP),
        "{label}: expected ComplexityLimitExceeded(limit=256), got {error:?}"
    );
    assert_eq!(
        error.gqlstatus(),
        GqlStatus::PROGRAM_LIMIT_EXCEEDED,
        "{label}"
    );
}

/// Nesting comfortably past the ~2264 pest-descent crash floor.
const HOSTILE_CASE_NESTING: usize = 5_000;

#[test]
fn deeply_nested_case_rejects_before_pest() {
    // `CASE WHEN true THEN CASE WHEN true THEN … ELSE 0 END … ELSE 0 END`.
    // All N `CASE`s open before any `END` closes, so case_depth climbs to N and
    // the guard rejects at 257 — long before pest's ~2264 overflow.
    let mut source = String::from("RETURN ");
    for _ in 0..HOSTILE_CASE_NESTING {
        source.push_str("CASE WHEN true THEN ");
    }
    source.push('0');
    for _ in 0..HOSTILE_CASE_NESTING {
        source.push_str(" ELSE 0 END");
    }
    assert_complexity_rejected(&source, "deeply nested CASE");
}

#[test]
fn nested_case_with_identifier_end_does_not_bypass() {
    // THE load-bearing soundness test (the PR #224 bypass). Each level carries a
    // `{END: 1}` map key in its WHEN condition: that `END` is a record-key
    // identifier (followed by `:`) and MUST NOT decrement case_depth. If it did,
    // the real `CASE` opens would be cancelled and the deep nesting would reach
    // pest and crash. The guard must still reject.
    let mut source = String::from("RETURN ");
    for _ in 0..HOSTILE_CASE_NESTING {
        source.push_str("CASE WHEN {END: 1} THEN ");
    }
    source.push('0');
    for _ in 0..HOSTILE_CASE_NESTING {
        source.push_str(" ELSE 0 END");
    }
    assert_complexity_rejected(&source, "nested CASE with {END:1} keys");
}

#[test]
fn nested_case_with_property_end_does_not_bypass() {
    // The `n.END` property-name variant of the bypass: `END` after `.` is an
    // identifier and must not decrement case_depth.
    let mut source = String::from("MATCH (n) RETURN ");
    for _ in 0..HOSTILE_CASE_NESTING {
        source.push_str("CASE WHEN n.END THEN ");
    }
    source.push('0');
    for _ in 0..HOSTILE_CASE_NESTING {
        source.push_str(" ELSE 0 END");
    }
    assert_complexity_rejected(&source, "nested CASE with n.END properties");
}

#[test]
fn lowercase_nested_case_rejects() {
    // Keywords are case-insensitive (`^"CASE"`/`^"END"`); a lowercase chain must
    // count too, or it bypasses.
    let mut source = String::from("RETURN ");
    for _ in 0..HOSTILE_CASE_NESTING {
        source.push_str("case when true then ");
    }
    source.push('0');
    for _ in 0..HOSTILE_CASE_NESTING {
        source.push_str(" else 0 end");
    }
    assert_complexity_rejected(&source, "lowercase nested case");
}

#[test]
fn real_case_expressions_parse() {
    parse("RETURN CASE WHEN true THEN 1 ELSE 0 END").expect("searched CASE parses");
    parse("RETURN CASE 1 WHEN 1 THEN 'a' WHEN 2 THEN 'b' ELSE 'c' END")
        .expect("simple CASE parses");
    // Modest nesting (depth 3) is legitimate and well under the cap.
    parse("RETURN CASE WHEN true THEN CASE WHEN false THEN CASE WHEN true THEN 1 ELSE 2 END ELSE 3 END ELSE 4 END")
        .expect("triple-nested CASE parses");
}

#[test]
fn case_and_end_as_identifiers_parse() {
    // `CASE`/`END` as `prop_ident` identifiers in every position the guard must
    // recognize and NOT count — none may be falsely rejected.
    // Property access (after `.`).
    parse("MATCH (n) RETURN n.END").expect("n.END property parses");
    parse("MATCH (n) RETURN n.CASE").expect("n.CASE property parses");
    // Map / record keys (followed by `:`).
    parse("RETURN {END: 1}").expect("{END: 1} record key parses");
    parse("RETURN {CASE: 1}").expect("{CASE: 1} record key parses");
    // Node-pattern property key.
    parse("MATCH (n {END: 1}) RETURN n").expect("node property key END parses");
    // Aliases (after AS).
    parse("RETURN 1 AS END").expect("AS END alias parses");
    parse("RETURN 1 AS CASE").expect("AS CASE alias parses");
    // Parameters (after `$`).
    parse("RETURN $CASE").expect("$CASE parameter parses");
    parse("RETURN $END").expect("$END parameter parses");
    // SET property name (after `.`).
    parse("MATCH (n) SET n.END = 1 FINISH").expect("SET n.END parses");
}

#[test]
fn unicode_identifier_with_keyword_tail_parses() {
    // `éCASE` / `éEND` are single Unicode identifiers (LETTER tail), NOT the
    // keyword — the guard must consume the whole word and not mis-segment the
    // ASCII tail. A long flat list must not accumulate phantom CASE/END depth.
    parse("RETURN éCASE").expect("éCASE is a Unicode variable, not the keyword");
    parse("RETURN éEND").expect("éEND is a Unicode variable");
    // 300 comma-separated `éCASE` RETURN items (separate exprs, no fold depth):
    // if `éCASE` were mis-segmented into `é` + `CASE`, each would count a phantom
    // `CASE` opener and the balanced counter would reject at 257. Correct Unicode
    // word segmentation keeps case_depth at 0, so all 300 parse.
    let list = vec!["éCASE"; 300].join(", ");
    parse(&format!("RETURN {list}")).expect("many éCASE identifiers do not accumulate case depth");
}

#[test]
fn whitespace_and_comments_do_not_defeat_identifier_recognition() {
    // pest's non-atomic `~` allows whitespace/comments between `prop_ident` and
    // its surrounding `.`/`:`. The lookbehind/lookahead must skip them, or these
    // identifier `END`/`CASE`s are mis-counted (and a hostile chain could hide a
    // decrement behind a comment). All must parse.
    parse("MATCH (n) RETURN n . END").expect("n . END (spaced) parses");
    parse("MATCH (n) RETURN n ./* c */ END").expect("n ./* */ END parses");
    parse("RETURN {END /* c */ : 1}").expect("{END /* */ : 1} parses");
    parse("RETURN {CASE   :   1}").expect("{CASE : 1} (spaced) parses");
}

#[test]
fn case_inside_function_arg_still_counts() {
    // A real `CASE` opener after `,` inside a function-argument list IS the
    // keyword (not an identifier) and must be bounded — confirm a deep such
    // nesting rejects rather than bypassing via the `,` predecessor.
    let mut source = String::from("RETURN coalesce(1, ");
    for _ in 0..HOSTILE_CASE_NESTING {
        source.push_str("CASE WHEN true THEN ");
    }
    source.push('0');
    for _ in 0..HOSTILE_CASE_NESTING {
        source.push_str(" ELSE 0 END");
    }
    source.push(')');
    assert_complexity_rejected(&source, "CASE nested inside function arg");
}
