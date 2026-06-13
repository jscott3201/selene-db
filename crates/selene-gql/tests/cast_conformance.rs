//! CONFORMANCE-00 acceptance bars — CAST conformance-honesty surface.
//!
//! Split out of `cast.rs` (which retains the core CAST parser/analyzer/runtime
//! ISO §22 dispatch-matrix coverage) to keep both test files under the
//! 700-LOC cap. These tests pin the *conformance claims* selene-db makes (or
//! deliberately does not make) about CAST:
//!   - CAST records GA05 "Cast specification" (ISO §20.8 / Annex D row 53), the
//!     real ISO optional feature for `<cast specification>` — NOT GE08, which
//!     is "Reference parameters" (§17.7 / row 77). Per ISO Annex A item 52 a
//!     conforming implementation may not contain a `<cast specification>`
//!     without GA05, so CAST is gated behind GA05, not baseline.
//!   - GA05 IS claimed in `SUPPORTED_FEATURES` (selene-db implements CAST);
//!     GE08 is NOT (reference parameters are unimplemented).
//!   - The positive CAST corpus parses/analyzes and stamps GA05.
//!   - The CHANGELOG `[Unreleased]` section documents the GA05 claim.

use selene_core::feature_register::FeatureId;
use selene_gql::{EmptyProcedureRegistry, analyze, feature_walk, parse};

fn parse_or_panic(source: &str) -> selene_gql::Statement {
    parse(source).unwrap_or_else(|err| panic!("parse failed for `{source}`: {err:?}"))
}

#[test]
fn cast_records_ga05_not_ge08() {
    // CONFORMANCE-00 (Codex review follow-up): a bare `CAST(<expr> AS <type>)`
    // records GA05 "Cast specification" (ISO §20.8 / Annex D row 53), the real
    // ISO optional feature for the cast construct — NOT GE08 ("Reference
    // parameters", §17.7 / row 77). For INTEGER → STRING no source/target type
    // contributes a feature, so GA05 is the only marker.
    let stmt = parse_or_panic("RETURN CAST(1 AS STRING) AS s");
    let features = feature_walk(&stmt)
        .into_iter()
        .map(|f| f.feature_id)
        .collect::<Vec<_>>();
    assert!(
        features.contains(&FeatureId::GA05),
        "CAST must record GA05 (Cast specification), observed {features:?}"
    );
    assert!(
        !features.contains(&FeatureId::GE08),
        "CAST must NOT record GE08 (Reference parameters), observed {features:?}"
    );
    assert_eq!(
        features,
        vec![FeatureId::GA05],
        "INTEGER → STRING CAST records exactly GA05, observed {features:?}"
    );
}

#[test]
fn cast_feature_ga05_claimed_ge08_not() {
    // CONFORMANCE-00 (Codex review follow-up): GA05 ("Cast specification",
    // §20.8) IS claimed — selene-db implements the cast construct and ISO Annex
    // A item 52 requires GA05 for any `<cast specification>`. The mislabeled
    // GE08 ("Reference parameters", §17.7) is NOT claimed (reference parameters
    // are unimplemented).
    use selene_core::feature_register::SUPPORTED_FEATURES;
    assert!(
        SUPPORTED_FEATURES.contains(&FeatureId::GA05),
        "FeatureId::GA05 (Cast specification) must be claimed — CAST is implemented"
    );
    assert!(
        !SUPPORTED_FEATURES.contains(&FeatureId::GE08),
        "FeatureId::GE08 (Reference parameters) must NOT be in SUPPORTED_FEATURES"
    );
}

#[test]
fn iso_conformance_cast_positive_corpus_all_pass() {
    // CONFORMANCE-00 (Codex review follow-up): the positive CAST corpus
    // (formerly GE08-cast*.gql, renamed to cast-*.gql + re-declared
    // `feature: GA05`) must parse, analyze, and record GA05 — the real ISO cast
    // feature — never the mislabeled GE08.
    use selene_testing::corpus::{CorpusKind, Expectation, load_default_corpus};

    let cases = load_default_corpus().expect("corpus loads");
    let cast_cases: Vec<_> = cases
        .iter()
        .filter(|case| {
            case.kind == CorpusKind::Positive
                && case.expectation == Expectation::ParseOk
                && case
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("cast-"))
        })
        .collect();
    assert!(
        cast_cases.len() >= 5,
        "expected at least 5 cast-*.gql positive corpus entries; found {}",
        cast_cases.len()
    );
    for case in cast_cases {
        let statement = parse(&case.source)
            .unwrap_or_else(|err| panic!("{} failed to parse: {err:?}", case.path.display()));
        let features = feature_walk(&statement)
            .into_iter()
            .map(|f| f.feature_id)
            .collect::<Vec<_>>();
        assert!(
            features.contains(&FeatureId::GA05) && !features.contains(&FeatureId::GE08),
            "{} must stamp GA05 (not GE08) for CAST; observed {features:?}",
            case.path.display()
        );
        analyze(statement, &EmptyProcedureRegistry, None)
            .unwrap_or_else(|err| panic!("{} failed to analyze: {err:?}", case.path.display()));
    }
}

#[test]
fn changelog_unreleased_documents_conformance_destamp() {
    // CONFORMANCE-00 (Codex review follow-up): release notes must document the
    // conformance-honesty fix — GE08 reclaimed for "Reference parameters"
    // (§17.7) and removed from CAST, GA05 "Cast specification" (§20.8) claimed
    // for CAST, and GG21 de-stamped (§18.2/18.3). Pin the entry so a forgotten
    // changelog edit fails the build, including after the entry moves from
    // [Unreleased] into the cut release section.
    let changelog = include_str!("../../../CHANGELOG.md");
    let release_notes = changelog
        .split("## [1.2.0]")
        .nth(1)
        .expect("CHANGELOG has a [1.2.0] section")
        // Stop at the next released-version heading so we only inspect the
        // release body.
        .split("\n## [")
        .next()
        .expect("release body");
    assert!(
        release_notes.contains("GE08"),
        "[1.2.0] must mention the GE08 de-stamp; observed: {release_notes}"
    );
    assert!(
        release_notes.contains("GA05"),
        "[1.2.0] must mention the GA05 claim; observed: {release_notes}"
    );
    assert!(
        release_notes.contains("GG21"),
        "[1.2.0] must mention the GG21 de-stamp; observed: {release_notes}"
    );
    assert!(
        release_notes.contains("Reference parameters"),
        "[1.2.0] must name GE08's real ISO meaning (Reference parameters)"
    );
}
