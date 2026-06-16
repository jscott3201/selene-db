//! Pre-pest nested-`CASE … END` depth guard coverage (node 818, Family A).
//!
//! `case_expr ↔ expr` is a zero-delimiter pest recursion: each nested
//! `CASE WHEN … THEN … END` re-enters the full precedence cascade for its
//! operand expressions, overflowing the native stack at ~2264 nesting (a ~53 KB
//! input) — a non-unwindable crash. The pre-pest byte-scan guard tracks a
//! **monotone** `case_depth`: a real `CASE` opener adds 1 plus the sign/`NOT`
//! run that wraps it, and it NEVER decrements. That sum (with the delimiter and
//! run counters) is a sound upper bound on the live native-stack depth, so
//! hostile nesting is rejected as `ComplexityLimitExceeded` (GQLSTATUS 5GQL1)
//! before pest runs.
//!
//! Why monotone (not balanced)? `CASE`/`END` are reserved but appear as
//! identifiers (via `prop_ident`) in property names, map/record keys, aliases,
//! `YIELD` columns, and parameters. A `CASE` *opener* is recognized soundly (it
//! is counted only outside identifier positions, which a real `case_expr` never
//! occupies). But the `END` *closer* cannot be told from an identifier `END`
//! (e.g. a comma-tail `YIELD a, END` column), so trusting it as a closer is a
//! confirmed bypass — a phantom identifier-`END` would cancel a real open `CASE`.
//! So the guard never decrements on `END`. The cost is a documented, conformant
//! limit (combined `CASE`-count + nesting pressure ≤ 256). These tests assert
//! (a) hostile nesting rejects, (b) the identifier-`END` bypasses (`{END:1}`,
//! `n.END`, the `YIELD … , END` comma-tail) and `NOT`-wrapped nesting are all
//! closed, (c) every identifier position still parses, and (d) the monotone
//! sequential-`CASE` limit holds at the boundary.

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
    // A load-bearing soundness test (the PR #224 bypass). Each level carries a
    // `{END: 1}` map key in its WHEN condition: that `END` is a record-key
    // identifier. Under the monotone guard `END` never decrements at all, so the
    // real `CASE` opens accumulate and the guard rejects — confirming no
    // identifier-`END` can cancel an open `CASE`.
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
    // The `n.END` property-name variant: `END` after `.` is an identifier. The
    // monotone guard never decrements on any `END`, so the real `CASE` opens
    // accumulate and the deep nesting is rejected.
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
fn case_and_end_identifier_slots_observe_keyword_boundaries() {
    // `CASE`/`END` as `prop_ident` identifiers in every position the guard must
    // recognize and NOT count -- none may be falsely rejected.
    // Property access (after `.`).
    parse("MATCH (n) RETURN n.END").expect("n.END property parses");
    parse("MATCH (n) RETURN n.CASE").expect("n.CASE property parses");
    // Map / record keys (followed by `:`).
    parse("RETURN {END: 1}").expect("{END: 1} record key parses");
    parse("RETURN {CASE: 1}").expect("{CASE: 1} record key parses");
    // Node-pattern property key.
    parse("MATCH (n {END: 1}) RETURN n").expect("node property key END parses");
    // Aliases are strict identifier slots; reserved words must be delimited.
    assert!(parse("RETURN 1 AS END").is_err());
    assert!(parse("RETURN 1 AS CASE").is_err());
    parse("RETURN 1 AS \"END\"").expect("delimited END alias parses");
    parse("RETURN 1 AS \"CASE\"").expect("delimited CASE alias parses");
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

#[test]
fn yield_tail_keyword_end_does_not_bypass() {
    // CONFIRMED PRE-FIX BYPASS (#1). `END` is admitted as a bare `prop_ident`
    // YIELD column (`CALL p() YIELD x, END`), and `VALUE { … }` lets that clause
    // sit inside a `CASE` operand. A *balanced* counter wrongly decremented on
    // that comma-tail `END` (the lookback sees `,`, not `YIELD`), pinning
    // case_depth at ~1 and letting arbitrarily deep real `CASE` nesting reach
    // pest and overflow. The monotone guard never decrements on `END`, so the
    // real `CASE` opens accumulate and it rejects before pest.
    let mut source = String::from("RETURN ");
    for _ in 0..HOSTILE_CASE_NESTING {
        source.push_str("CASE WHEN VALUE { CALL p() YIELD x, END RETURN true } THEN ");
    }
    source.push('0');
    for _ in 0..HOSTILE_CASE_NESTING {
        source.push_str(" ELSE 0 END");
    }
    assert_complexity_rejected(&source, "nested CASE hiding YIELD-tail END");
}

#[test]
fn not_wrapped_nested_case_does_not_bypass() {
    // CONFIRMED PRE-FIX BYPASS (#2). A run of `NOT` WRAPS each nested `CASE`
    // (operand position), so the outer `NOT` frames stay open while inner CASEs
    // parse; resetting the run counter at each `CASE` undercounts the true
    // descent depth (`levels × (run + 1)`). The monotone guard folds the wrapping
    // run into case_depth at each opener, so the sum tracks the true depth and
    // rejects before pest.
    const NOT_RUN: usize = 120;
    let nots = "NOT ".repeat(NOT_RUN);
    let mut source = String::from("RETURN ");
    for _ in 0..HOSTILE_CASE_NESTING {
        source.push_str(&nots);
        source.push_str("CASE WHEN true THEN ");
    }
    source.push('0');
    for _ in 0..HOSTILE_CASE_NESTING {
        source.push_str(" ELSE 0 END");
    }
    assert_complexity_rejected(&source, "NOT-wrapped nested CASE");
}

#[test]
fn sequential_case_expressions_within_cap_parse() {
    // The monotone counter accumulates across SEQUENTIAL (non-nested) `CASE`
    // expressions too. A statement exactly at the cap still parses:
    // RECURSION_DEPTH_CAP independent shallow `CASE` columns drive case_depth to
    // exactly the cap, which is admitted (the budget rejects only when exceeded).
    let items: Vec<String> = (0..RECURSION_DEPTH_CAP)
        .map(|i| format!("CASE WHEN true THEN 1 ELSE 0 END AS c{i}"))
        .collect();
    let source = format!("RETURN {}", items.join(", "));
    parse(&source).expect("RECURSION_DEPTH_CAP sequential CASE columns parse");
}

#[test]
fn excessive_sequential_case_expressions_reject() {
    // One past the cap: the documented monotone tradeoff. A pathological
    // statement with more than RECURSION_DEPTH_CAP `CASE` expressions (even
    // entirely non-nested) is rejected with the program-limit GQLSTATUS — the
    // conformant cost of never decrementing on `END`.
    let items: Vec<String> = (0..=RECURSION_DEPTH_CAP)
        .map(|i| format!("CASE WHEN true THEN 1 ELSE 0 END AS c{i}"))
        .collect();
    let source = format!("RETURN {}", items.join(", "));
    assert_complexity_rejected(&source, "RECURSION_DEPTH_CAP+1 sequential CASE columns");
}
