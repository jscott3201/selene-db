//! PARSE-14: `parse_many` enforces the interner-admission budget and the
//! syntactic nesting guard PER-SEGMENT, not per-call.
//!
//! Each `;`-separated segment is handed to `parse`, which builds a fresh
//! `InternerBudget` and runs the nesting `guard` over that segment alone. These
//! tests pin that contract: the budget resets at every statement boundary, an
//! over-budget segment is rejected with a span rebased into the original
//! multi-statement source, and the nesting guard fires per statement.

use std::sync::atomic::{AtomicU64, Ordering};

use selene_gql::{ParserError, parse_many};

/// Per-parse interner admission cap (mirrors `parser::budget`).
const LIMIT: usize = 8_192;
/// Per-parse syntactic nesting cap (mirrors `parser::guard`).
const NESTING_LIMIT: usize = 64;

/// Process-unique prefix so distinct test runs never collide in the global
/// interner (admission-aware: the DoS budget only charges genuinely new
/// strings, so reused names from a prior parse would not count).
fn unique_prefix(name: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "pm_{name}_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// `RETURN <prefix>_0, <prefix>_1, ...` admitting `count` distinct identifiers.
fn return_identifiers(prefix: &str, count: usize) -> String {
    let items = (0..count)
        .map(|index| format!("{prefix}_{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("RETURN {items}")
}

#[test]
fn budget_resets_per_segment_so_two_cap_sized_statements_both_parse() {
    // Two statements that each admit exactly LIMIT distinct identifiers. If the
    // budget were per-CALL the second statement would push the running total to
    // 2*LIMIT and be rejected; per-SEGMENT, each gets a fresh budget.
    let first = return_identifiers(&unique_prefix("seg_a"), LIMIT);
    let second = return_identifiers(&unique_prefix("seg_b"), LIMIT);
    let source = format!("{first}; {second}");

    let statements = parse_many(&source).expect("per-segment budget gives each statement LIMIT");
    assert_eq!(statements.len(), 2);
}

#[test]
fn over_budget_segment_is_rejected_with_a_rebased_span() {
    // A small valid leading statement, then a second statement that admits
    // LIMIT+1 distinct identifiers. The rejection must be InternerBudgetExceeded
    // with the documented limit, and its span must be rebased PAST the first
    // statement (proving per-segment enforcement + span rebasing).
    let lead = "RETURN 1 AS ok";
    let offender = return_identifiers(&unique_prefix("over"), LIMIT + 1);
    let source = format!("{lead}; {offender}");
    let second_start = source.find("RETURN ").unwrap();
    let offender_start = source[second_start + 1..].find("RETURN ").unwrap() + second_start + 1;

    let error = parse_many(&source).expect_err("the second segment exceeds its own budget");
    let ParserError::InternerBudgetExceeded { limit, span } = error else {
        panic!("expected InternerBudgetExceeded, got {error:?}");
    };
    assert_eq!(limit, LIMIT as u32);
    assert!(
        span.byte_offset >= offender_start as u32,
        "span {span:?} should be rebased into the over-budget second statement (>= {offender_start})"
    );
    assert!(
        span.byte_offset < source.len() as u32,
        "span {span:?} must stay within the original source"
    );
}

#[test]
fn budget_consumed_by_first_segment_does_not_starve_a_later_segment() {
    // The first statement spends nearly the whole budget; a later statement that
    // would overflow a *shared* budget but fits its own LIMIT must still parse.
    let heavy = return_identifiers(&unique_prefix("heavy"), LIMIT);
    let trailing = return_identifiers(&unique_prefix("light"), LIMIT - 1);
    let source = format!("{heavy}; RETURN 1 AS mid; {trailing}");

    let statements = parse_many(&source).expect("later segment has its own fresh budget");
    assert_eq!(statements.len(), 3);
}

#[test]
fn nesting_guard_fires_per_segment_with_a_rebased_span() {
    // A valid leading statement, then a second statement whose bracket nesting
    // exceeds the per-parse cap. The guard runs over the second segment alone
    // and rejects with NestingLimitExceeded, span rebased past the first.
    let lead = "RETURN 1 AS ok";
    let deep = format!(
        "LET x = {}0{} RETURN x",
        "[".repeat(NESTING_LIMIT + 1),
        "]".repeat(NESTING_LIMIT + 1)
    );
    let source = format!("{lead}; {deep}");
    let deep_start = source.find("LET ").unwrap();

    let error = parse_many(&source).expect_err("the second segment is over-nested");
    let ParserError::NestingLimitExceeded { limit, span } = error else {
        panic!("expected NestingLimitExceeded, got {error:?}");
    };
    assert_eq!(limit, NESTING_LIMIT as u32);
    assert!(
        span.byte_offset >= deep_start as u32,
        "span {span:?} should be rebased into the over-nested second statement (>= {deep_start})"
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
