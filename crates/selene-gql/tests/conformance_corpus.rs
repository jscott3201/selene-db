//! Header-driven parser conformance corpus.

use std::collections::BTreeSet;

use selene_core::feature_register::{NOT_SUPPORTED_RATIONALE, SUPPORTED_FEATURES};
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
fn corpus_covers_feature_register() {
    let cases = load_default_corpus().expect("corpus loads");
    let positive = cases
        .iter()
        .filter(|case| case.kind == CorpusKind::Positive)
        .flat_map(|case| case.declared_features())
        .collect::<BTreeSet<_>>();
    // Some claimed features have no independent parser surface and are only
    // reachable behind an unclaimed feature that must reject first.
    let blocked_supported = cases
        .iter()
        .filter(|case| case.kind == CorpusKind::Negative)
        .flat_map(|case| case.also_covers.iter().copied())
        .collect::<BTreeSet<_>>();
    let negative = cases
        .iter()
        .filter(|case| case.kind == CorpusKind::Negative)
        .flat_map(|case| case.declared_features())
        .collect::<BTreeSet<_>>();

    let missing_supported = SUPPORTED_FEATURES
        .iter()
        .copied()
        .filter(|feature| !positive.contains(feature) && !blocked_supported.contains(feature))
        .collect::<Vec<_>>();
    assert!(
        missing_supported.is_empty(),
        "missing positive corpus coverage for {:?}",
        missing_supported
            .iter()
            .map(|feature| feature.as_str())
            .collect::<Vec<_>>()
    );

    let missing_rejected = NOT_SUPPORTED_RATIONALE
        .iter()
        .map(|(feature, _)| *feature)
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
