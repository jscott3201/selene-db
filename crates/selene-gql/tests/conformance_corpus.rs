//! Header-driven parser conformance corpus.

use std::collections::BTreeSet;

use selene_core::feature_register::{FLAGGER_ACCEPTED_FEATURES, NOT_SUPPORTED_RATIONALE};
use selene_gql::{ParserError, feature_walk, parse};
use selene_testing::corpus::{CorpusKind, Expectation, load_default_corpus};

#[test]
fn corpus_contracts_hold() {
    let cases = load_default_corpus().expect("corpus loads");
    assert!(!cases.is_empty(), "corpus must not be empty");

    for case in &cases {
        match (case.kind, case.expectation) {
            (CorpusKind::Positive, Expectation::ParseOk) => {
                let statement = parse(&case.source).unwrap_or_else(|error| {
                    panic!("{}: expected parse-ok, got {error:?}", case.path.display())
                });
                let declared_features = case.declared_features().collect::<Vec<_>>();
                if declared_features.is_empty() {
                    continue;
                }
                let observed = feature_walk(&statement)
                    .into_iter()
                    .map(|feature| feature.feature_id)
                    .collect::<BTreeSet<_>>();
                for feature in declared_features {
                    assert!(
                        observed.contains(&feature),
                        "{}: declared feature {feature} was not observed; observed {:?}",
                        case.path.display(),
                        observed
                            .iter()
                            .map(|feature| feature.as_str())
                            .collect::<Vec<_>>()
                    );
                }
            }
            (CorpusKind::Negative, Expectation::ParseRejectedFeature(expected)) => {
                let error = parse(&case.source).unwrap_err_or_else(|| {
                    panic!("{}: expected feature rejection", case.path.display())
                });
                let ParserError::UnsupportedFeature { feature_id, .. } = error else {
                    panic!(
                        "{}: expected UnsupportedFeature({expected}), got {error:?}",
                        case.path.display()
                    );
                };
                assert_eq!(feature_id, expected, "{}", case.path.display());
            }
            (CorpusKind::Negative, Expectation::ParseRejectedSyntax) => {
                let error = parse(&case.source).unwrap_err_or_else(|| {
                    panic!("{}: expected syntax rejection", case.path.display())
                });
                assert!(
                    matches!(
                        error,
                        ParserError::SyntaxError { .. } | ParserError::NotImplemented { .. }
                    ),
                    "{}: expected syntax or NotImplemented rejection, got {error:?}",
                    case.path.display()
                );
            }
            (CorpusKind::Positive, _) | (CorpusKind::Negative, Expectation::ParseOk) => {
                panic!("{}: invalid corpus contract", case.path.display());
            }
        }
    }
}

#[test]
fn corpus_covers_flagger_compatibility_set() {
    let cases = load_default_corpus().expect("corpus loads");
    let positive = cases
        .iter()
        .filter(|case| case.kind == CorpusKind::Positive)
        .flat_map(|case| case.declared_features())
        .collect::<BTreeSet<_>>();
    // Some accepted features have no independent parser surface and are only
    // reachable behind a feature that must reject first.
    let blocked_accepted = cases
        .iter()
        .filter(|case| case.kind == CorpusKind::Negative)
        .flat_map(|case| case.also_covers.iter().copied())
        .collect::<BTreeSet<_>>();
    let negative = cases
        .iter()
        .filter(|case| case.kind == CorpusKind::Negative)
        .flat_map(|case| case.declared_features())
        .collect::<BTreeSet<_>>();

    let missing_accepted = FLAGGER_ACCEPTED_FEATURES
        .iter()
        .copied()
        .filter(|feature| !positive.contains(feature) && !blocked_accepted.contains(feature))
        .collect::<Vec<_>>();
    assert!(
        missing_accepted.is_empty(),
        "missing parser-accepted corpus coverage for {:?}",
        missing_accepted
            .iter()
            .map(|feature| feature.as_str())
            .collect::<Vec<_>>()
    );

    let missing_rejected = NOT_SUPPORTED_RATIONALE
        .iter()
        .map(|(feature, _)| *feature)
        .filter(|feature| !FLAGGER_ACCEPTED_FEATURES.contains(feature))
        .filter(|feature| !negative.contains(feature))
        .collect::<Vec<_>>();
    assert!(
        missing_rejected.is_empty(),
        "missing negative corpus coverage for {:?}",
        missing_rejected
            .iter()
            .map(|feature| feature.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn canonical_cases_observe_exactly_their_curated_feature_set() {
    // PARSE-16: `corpus_contracts_hold` only checks declared ⊆ observed, which
    // catches a feature the parser stops stamping but NOT over-stamping — a
    // spurious extra feature stamp would cause a spurious UnsupportedFeature
    // rejection (ISO §24.6). Most corpus headers intentionally declare a SUBSET
    // of observed features (minimum-conformance + implied features stay
    // undeclared), so a blanket observed==declared assertion is wrong. Instead,
    // pin a curated set of canonical cases to their COMPLETE observed feature
    // multiset. A regression that adds or drops any feature stamp on these
    // stable shapes fails this exact-equality check.
    //
    // Each entry is (corpus filename, complete sorted-unique observed feature
    // id set). Captured from `feature_walk` over the parsed source.
    let expected: &[(&str, &[&str])] = &[
        // Single-feature minimum-surface cases.
        ("G010-walk-explicit.gql", &["G010"]),
        ("G100-element-id.gql", &["G100"]),
        ("G110-is-directed.gql", &["G110"]),
        ("G111-is-labeled.gql", &["G111"]),
        ("G113-all-different.gql", &["G113"]),
        ("G114-same.gql", &["G114"]),
        ("G115-property-exists.gql", &["G115"]),
        ("GA01-ieee754-arithmetic.gql", &["GA01"]),
        ("GA06-value-type-predicate.gql", &["GA06"]),
        ("GE07-xor.gql", &["GE07"]),
        ("GH02-undirected-edge-pattern.gql", &["GH02"]),
        ("GQ02-otherwise.gql", &["GQ02"]),
        ("GQ03-union.gql", &["GQ03"]),
        ("GQ08-filter.gql", &["GQ08"]),
        ("GQ09-let.gql", &["GQ09"]),
        ("GQ15-group-by.gql", &["GQ15"]),
        ("GQ20-next.gql", &["GQ20"]),
        // Multi-feature shapes where the COMPLETE observed set matters (these
        // are exactly the cases an over-stamping regression would corrupt).
        ("G011-trail.gql", &["G011", "G036", "G060"]),
        ("G037-questioned-edge.gql", &["G036", "G037"]),
        ("GE04-parameters.gql", &["GE04", "GE05"]),
        ("GF01-enhanced-numeric.gql", &["GA01", "GF01"]),
        (
            "GL04-GL10-numeric-literal-source-forms.gql",
            &[
                "GA01", "GL04", "GL05", "GL06", "GL07", "GL08", "GL09", "GL10", "GV17",
            ],
        ),
        ("GP01-inline-procedure.gql", &["GP01", "GP02", "GQ09"]),
        ("GQ18-value-subquery.gql", &["GQ13", "GQ18"]),
        ("GV45-record-literal.gql", &["GV45", "GV50"]),
        ("GV46-closed-record-type.gql", &["GA06", "GV45", "GV46"]),
        (
            "GV48-nested-record-type.gql",
            &["GA06", "GV45", "GV46", "GV48"],
        ),
        // CONFORMANCE-00 (Codex follow-up): CAST records GA05 "Cast
        // specification" (ISO Annex A item 52 / §20.8 — GE08 is "Reference
        // parameters" per §17.7, not CAST), and the UUID target type records
        // IM_UUID; both are observed here.
        ("IMU-uuid-cast.gql", &["GA05", "IM_UUID"]),
        ("IMJ-json-functions.gql", &["IM_JSON"]),
    ];

    let cases = load_default_corpus().expect("corpus loads");
    let mut matched = 0usize;
    for (file_name, expected_features) in expected {
        let case = cases
            .iter()
            .find(|case| {
                case.path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == *file_name)
            })
            .unwrap_or_else(|| panic!("curated corpus case {file_name} not found"));
        assert_eq!(
            case.kind,
            CorpusKind::Positive,
            "{file_name} must be positive"
        );

        let statement = parse(&case.source)
            .unwrap_or_else(|error| panic!("{file_name}: expected parse-ok, got {error:?}"));
        let observed = feature_walk(&statement)
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<BTreeSet<_>>();
        let observed_names = observed
            .iter()
            .map(|feature| feature.as_str())
            .collect::<Vec<_>>();
        let expected_names = expected_features.iter().copied().collect::<BTreeSet<_>>();
        let expected_sorted = expected_names.into_iter().collect::<Vec<_>>();

        assert_eq!(
            observed_names, expected_sorted,
            "{file_name}: observed feature multiset drifted from the curated exact set \
             (over- or under-stamping)"
        );
        matched += 1;
    }
    assert_eq!(matched, expected.len(), "all curated cases must be present");
}

trait ResultExt<T, E> {
    fn unwrap_err_or_else(self, f: impl FnOnce() -> E) -> E;
}

impl<T, E> ResultExt<T, E> for Result<T, E> {
    fn unwrap_err_or_else(self, f: impl FnOnce() -> E) -> E {
        match self {
            Ok(_) => f(),
            Err(error) => error,
        }
    }
}
