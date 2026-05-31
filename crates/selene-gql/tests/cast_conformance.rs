//! CONFORMANCE-00 acceptance bars — CAST conformance-honesty surface.
//!
//! Split out of `cast.rs` (which retains the core CAST parser/analyzer/runtime
//! ISO §22 dispatch-matrix coverage) to keep both test files under the
//! 700-LOC cap. These tests pin the *conformance claims* selene-db makes (or
//! deliberately does not make) about CAST:
//!   - CAST records NO ISO optional feature — GE08 is "Reference parameters"
//!     (ISO §17.7 / Annex D row 77), not CAST; the real cast feature is GA05
//!     "Cast specification" (§20.8 / row 53), which selene-db does not gate
//!     CAST behind (CAST ships as ungated baseline value-expression surface).
//!   - Neither GE08 nor GA05 is claimed in `SUPPORTED_FEATURES`.
//!   - The positive CAST corpus parses/analyzes and stamps neither feature.
//!   - The CHANGELOG `[Unreleased]` section documents the de-stamp rationale.

use selene_core::feature_register::FeatureId;
use selene_gql::{EmptyProcedureRegistry, analyze, feature_walk, parse};

fn parse_or_panic(source: &str) -> selene_gql::Statement {
    parse(source).unwrap_or_else(|err| panic!("parse failed for `{source}`: {err:?}"))
}

#[test]
fn cast_records_no_iso_optional_feature() {
    // CONFORMANCE-00: a bare `CAST(<expr> AS <type>)` records NO ISO optional
    // feature. GE08 is "Reference parameters" (ISO §17.7 / Annex D row 77), not
    // CAST; the real cast feature is GA05 "Cast specification" (§20.8 / row 53),
    // which selene-db does not gate CAST behind. CAST ships as baseline value-
    // expression surface, so the only features that may appear are the ones
    // contributed by the source/target TYPE (none for INTEGER → STRING).
    let stmt = parse_or_panic("RETURN CAST(1 AS STRING) AS s");
    let features = feature_walk(&stmt)
        .into_iter()
        .map(|f| f.feature_id)
        .collect::<Vec<_>>();
    assert!(
        !features.contains(&FeatureId::GE08),
        "CAST must NOT record GE08 (Reference parameters), observed {features:?}"
    );
    assert!(
        !features.contains(&FeatureId::GA05),
        "CAST does not stamp GA05 today (baseline surface), observed {features:?}"
    );
    assert!(
        features.is_empty(),
        "INTEGER → STRING CAST contributes no optional-feature marker, observed {features:?}"
    );
}

#[test]
fn cast_features_ge08_and_ga05_are_not_claimed() {
    // CONFORMANCE-00: neither the mislabeled GE08 ("Reference parameters",
    // §17.7) nor the real cast feature GA05 ("Cast specification", §20.8) is
    // claimed as supported. GE08 is unimplemented (no reference parameters);
    // GA05 is reserved because CAST ships as ungated baseline surface.
    use selene_core::feature_register::SUPPORTED_FEATURES;
    assert!(
        !SUPPORTED_FEATURES.contains(&FeatureId::GE08),
        "FeatureId::GE08 (Reference parameters) must NOT be in SUPPORTED_FEATURES"
    );
    assert!(
        !SUPPORTED_FEATURES.contains(&FeatureId::GA05),
        "FeatureId::GA05 (Cast specification) is reserved, not yet claimed"
    );
}

#[test]
fn iso_conformance_cast_positive_corpus_all_pass() {
    // CONFORMANCE-00: the positive CAST corpus (formerly the GE08-cast*.gql
    // files, renamed to cast-*.gql + re-declared `feature: none`) must parse,
    // analyze, and record NO mislabeled GE08 / reserved GA05 stamp. CAST is
    // baseline value-expression surface, not an ISO optional feature.
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
            !features.contains(&FeatureId::GE08) && !features.contains(&FeatureId::GA05),
            "{} must not stamp GE08/GA05 for CAST; observed {features:?}",
            case.path.display()
        );
        analyze(statement, &EmptyProcedureRegistry, None)
            .unwrap_or_else(|err| panic!("{} failed to analyze: {err:?}", case.path.display()));
    }
}

#[test]
fn changelog_unreleased_documents_conformance_destamp() {
    // CONFORMANCE-00: the [Unreleased] section must document the conformance-
    // honesty fix — GE08 is reclaimed for "Reference parameters" (§17.7) and no
    // longer stamped on CAST, and GG21 is de-stamped (§18.2/18.3). Pin the
    // entry so a forgotten changelog edit fails the build, not release prep.
    let changelog = include_str!("../../../CHANGELOG.md");
    let unreleased = changelog
        .split("## [Unreleased]")
        .nth(1)
        .expect("CHANGELOG has an [Unreleased] section")
        // Stop at the first released-version heading so we only inspect the
        // Unreleased body.
        .split("\n## [")
        .next()
        .expect("Unreleased body");
    assert!(
        unreleased.contains("GE08"),
        "[Unreleased] must mention the GE08 de-stamp; observed: {unreleased}"
    );
    assert!(
        unreleased.contains("GG21"),
        "[Unreleased] must mention the GG21 de-stamp; observed: {unreleased}"
    );
    assert!(
        unreleased.contains("Reference parameters"),
        "[Unreleased] must name GE08's real ISO meaning (Reference parameters)"
    );
}
