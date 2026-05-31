//! Per-parse interner admission budget coverage.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use selene_gql::{ParserError, parse};

const LIMIT: usize = 8_192;
const NESTING_LIMIT: usize = 64;

fn unique_prefix(name: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "b19_{name}_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn return_identifiers(prefix: &str, count: usize) -> String {
    let items = (0..count)
        .map(|index| format!("{prefix}_{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("RETURN {items}")
}

fn return_string_literals(prefix: &str, count: usize) -> String {
    let items = (0..count)
        .map(|index| format!("'{prefix}_{index}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("RETURN {items}")
}

fn return_uuid_literals(count: usize) -> String {
    let items = (0..count)
        .map(|index| format!("UUID '00000000-0000-4000-8000-{index:012x}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("RETURN {items}")
}

fn is_typed_record_with_fields(prefix: &str, count: usize) -> String {
    let fields = (0..count)
        .map(|index| format!("{prefix}_{index} :: INTEGER"))
        .collect::<Vec<_>>()
        .join(", ");
    // The operand is an unbound variable: parse-time admission does not resolve bindings,
    // so each `<prefix>_<n>` RECORD field NAME is interned through the per-parse budget.
    format!("RETURN x IS TYPED RECORD{{{fields}}}")
}

#[test]
fn within_budget_parse_succeeds() {
    let source = return_identifiers(&unique_prefix("within"), LIMIT);
    parse(&source).expect("exactly-at-cap admission succeeds");
}

#[test]
fn rejects_identifier_admission_over_budget() {
    let source = return_identifiers(&unique_prefix("ident"), LIMIT + 1);
    let error = parse(&source).expect_err("admission over cap rejects");
    assert!(matches!(
        error,
        ParserError::InternerBudgetExceeded { limit: 8192, .. }
    ));
}

#[test]
fn string_literals_count_against_budget() {
    let source = return_string_literals(&unique_prefix("string"), LIMIT + 1);
    let error = parse(&source).expect_err("string admissions count against budget");
    assert!(matches!(
        error,
        ParserError::InternerBudgetExceeded { limit: 8192, .. }
    ));
}

#[test]
fn uuid_literals_do_not_count_against_string_budget() {
    let source = return_uuid_literals(LIMIT + 1);
    parse(&source).expect("UUID literals are not interned string literals");
}

#[test]
fn repeated_identifiers_do_not_count_after_first_admission() {
    let name = unique_prefix("repeat");
    let items = std::iter::repeat_n(name.as_str(), 100_000)
        .collect::<Vec<_>>()
        .join(", ");
    let source = format!("RETURN {items}");
    parse(&source).expect("repeated handle is free after first admission");
}

#[test]
fn cap_is_per_parse_not_per_process() {
    let first = return_identifiers(&unique_prefix("first"), LIMIT);
    let second = return_identifiers(&unique_prefix("second"), LIMIT);
    parse(&first).expect("first cap-sized parse succeeds");
    parse(&second).expect("second cap-sized parse has a fresh budget");
}

#[test]
fn over_budget_parse_does_not_pollute_global_interner() {
    // The 8193rd unique string in a parse must NOT enter the process-wide
    // interner: the local budget is checked BEFORE the global admission so
    // a rejected parse leaves the global pool unchanged.
    let prefix = unique_prefix("nopoll");
    let canary_index = LIMIT;
    let canary = format!("{prefix}_{canary_index}");
    assert!(
        selene_core::lookup(&canary).is_none(),
        "canary string is unexpectedly present before parse: {canary}"
    );

    let source = return_identifiers(&prefix, LIMIT + 1);
    let error = parse(&source).expect_err("over-budget parse rejects");
    assert!(matches!(
        error,
        ParserError::InternerBudgetExceeded { limit: 8192, .. }
    ));

    assert!(
        selene_core::lookup(&canary).is_none(),
        "rejected over-budget parse leaked canary into the global interner: {canary}"
    );
}

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
fn record_field_names_count_against_budget() {
    // RECORD type field names are a user-controlled admission surface (the RECORD arm of
    // build_type_name_with_depth interns each field name via the budget). LIMIT+1 globally
    // unique field names must exhaust the per-parse budget like any other admission.
    let source = is_typed_record_with_fields(&unique_prefix("recfield"), LIMIT + 1);
    let error = parse(&source).expect_err("record field-name admissions count against budget");
    assert!(matches!(
        error,
        ParserError::InternerBudgetExceeded { limit: 8192, .. }
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
fn no_unbudgeted_intern_call_in_selene_gql() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    collect_unbudgeted_calls(&src, &mut offenders);
    assert!(
        offenders.is_empty(),
        "direct intern calls bypass parser budget:\n{}",
        offenders
            .iter()
            .map(|path| path.display())
            .map(|display| display.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

fn collect_unbudgeted_calls(path: &Path, offenders: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(path).expect("read source directory") {
        let entry = entry.expect("read source entry");
        let path = entry.path();
        if path.is_dir() {
            collect_unbudgeted_calls(&path, offenders);
            continue;
        }
        if path.extension().is_some_and(|extension| extension == "rs") {
            let content = fs::read_to_string(&path).expect("read source file");
            if content.contains("intern(") {
                offenders.push(path);
            }
        }
    }
}
