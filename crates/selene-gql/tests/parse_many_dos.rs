//! PARSE-14: `parse_many` enforces the syntactic nesting guard PER-SEGMENT,
//! not per-call.
//!
//! Each `;`-separated segment is handed to `parse`, which runs the nesting
//! `guard` over that segment alone. These tests pin that contract: an
//! over-complex segment is rejected with a span rebased into the original
//! multi-statement source, and the nesting guard fires per statement.

use std::sync::atomic::{AtomicU64, Ordering};

use selene_gql::{ParserError, parse_many};

/// Per-parse syntactic nesting cap (mirrors `parser::guard`).
const NESTING_LIMIT: usize = 64;

/// Process-unique prefix so distinct test runs never collide.
fn unique_prefix(name: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "pm_{name}_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// `RETURN <prefix>_0, <prefix>_1, ...` building `count` identifiers.
fn return_identifiers(prefix: &str, count: usize) -> String {
    let items = (0..count)
        .map(|index| format!("{prefix}_{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("RETURN {items}")
}

#[test]
fn two_statements_both_parse() {
    let first = return_identifiers(&unique_prefix("seg_a"), 64);
    let second = return_identifiers(&unique_prefix("seg_b"), 64);
    let source = format!("{first}; {second}");

    let statements = parse_many(&source).expect("each statement parses independently");
    assert_eq!(statements.len(), 2);
}

#[test]
fn complexity_guard_fires_per_segment_with_a_rebased_span() {
    // A valid leading statement, then a second statement whose bracket-opener
    // run exceeds the per-statement complexity cap. The byte-scan guard runs
    // over the second segment alone and rejects with ComplexityLimitExceeded,
    // span rebased past the first. (A balanced run long enough to exceed the
    // 64-delimiter nesting cap exceeds the tighter complexity cap first.)
    let lead = "RETURN 1 AS ok";
    let deep = format!(
        "LET x = {}0{} RETURN x",
        "[".repeat(NESTING_LIMIT + 1),
        "]".repeat(NESTING_LIMIT + 1)
    );
    let source = format!("{lead}; {deep}");
    let deep_start = source.find("LET ").unwrap();

    let error = parse_many(&source).expect_err("the second segment is over-complex");
    let ParserError::ComplexityLimitExceeded { span, .. } = error else {
        panic!("expected ComplexityLimitExceeded, got {error:?}");
    };
    assert!(
        span.byte_offset >= deep_start as u32,
        "span {span:?} should be rebased into the over-complex second statement (>= {deep_start})"
    );
}

#[test]
fn nesting_guard_is_per_segment_not_summed_across_segments() {
    // Ten segments, each nested to a modest depth. The SUMMED depth (120) far
    // exceeds the per-parse cap (64); per-SEGMENT each is well under it, so all
    // ten parse. This pins that the guard does NOT carry bracket depth across
    // `;` boundaries. (Depth is kept modest so the pest recursive descent over
    // genuine nested list literals stays within the debug-build stack.)
    let per_segment_depth = 12;
    let segments = 10;
    assert!(
        per_segment_depth < NESTING_LIMIT,
        "each segment stays under cap"
    );
    assert!(
        per_segment_depth * segments > NESTING_LIMIT,
        "summed depth would exceed the cap if the guard accumulated across segments"
    );
    let one = format!(
        "RETURN {}0{}",
        "[".repeat(per_segment_depth),
        "]".repeat(per_segment_depth)
    );
    let source = std::iter::repeat_n(one.as_str(), segments)
        .collect::<Vec<_>>()
        .join("; ");

    let statements = parse_many(&source).expect("each segment's depth is bounded independently");
    assert_eq!(statements.len(), segments);
}

/// `guard::MAX_RECURSION_DEPTH` mirror (not re-exported); a run one deeper is
/// rejected pre-pest, far below the ~2 MB-thread pest overflow floor.
const RECURSION_CAP: usize = 256;

#[test]
fn hostile_recursion_statement_in_multi_statement_input_rejected_per_segment() {
    // BRIEF 01b: a single hostile zero-delimiter recursion statement embedded in
    // a multi-statement program must be rejected by the per-segment guard
    // (ComplexityLimitExceeded, span rebased into the offending segment), never
    // crash the recursive descent. The over-cap count is safely below the
    // ~2 MB-thread pest overflow floor so this test cannot SIGABRT the runner.
    let over_cap = RECURSION_CAP + 64;
    let lead = "RETURN 1 AS ok";
    let chain = "NOT ".repeat(over_cap);
    let hostile = format!("RETURN {chain}true");
    let source = format!("{lead}; {hostile}");
    let hostile_start = source.find("RETURN N").unwrap();

    let error = parse_many(&source).expect_err("the second segment is over-recursive");
    let ParserError::ComplexityLimitExceeded { span, .. } = error else {
        panic!("expected ComplexityLimitExceeded, got {error:?}");
    };
    assert!(
        span.byte_offset >= hostile_start as u32,
        "span {span:?} should be rebased into the hostile second statement (>= {hostile_start})"
    );
}
