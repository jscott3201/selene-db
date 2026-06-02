//! BRIEF-125 conformance integrity regression coverage.

use selene_core::{
    CoreError, NodeId,
    feature_register::{FeatureId, NOT_SUPPORTED_RATIONALE, SUPPORTED_FEATURES},
    gqlstatus_name,
};
use selene_gql::{GqlStatus, ParserError, parse};
use selene_graph::GraphError;
use selene_persist::PersistError;

#[test]
fn unclaimed_graph_management_surfaces_emit_42n01() {
    let source = "CREATE GRAPH demo";
    let error = parse(source).expect_err(source);
    assert_eq!(error.gqlstatus().as_str(), "42N01");
    let ParserError::UnsupportedFeature { feature_id, .. } = error else {
        panic!("expected UnsupportedFeature for {source:?}");
    };
    assert_eq!(feature_id, FeatureId::GC04);
}

#[test]
fn path_mode_features_are_claimed_supported() {
    for feature in [
        FeatureId::G010,
        FeatureId::G011,
        FeatureId::G012,
        FeatureId::G013,
    ] {
        assert!(SUPPORTED_FEATURES.contains(&feature));
        assert!(
            !NOT_SUPPORTED_RATIONALE
                .iter()
                .any(|(unsupported, _)| *unsupported == feature)
        );
    }
}

#[test]
fn record_type_features_are_claimed_supported() {
    // GV45 (record types umbrella / value form) shipped earlier; GV46 (closed), GV47
    // (open), GV48 (nested) record TYPES ship with the typed/closed RECORD grammar.
    for feature in [
        FeatureId::GV45,
        FeatureId::GV46,
        FeatureId::GV47,
        FeatureId::GV48,
    ] {
        assert!(
            SUPPORTED_FEATURES.contains(&feature),
            "{feature} must be claimed supported"
        );
        assert!(
            !NOT_SUPPORTED_RATIONALE
                .iter()
                .any(|(unsupported, _)| *unsupported == feature),
            "{feature} must not remain in NOT_SUPPORTED_RATIONALE"
        );
    }
}

#[test]
fn deferred_reference_value_type_features_remain_unsupported() {
    // GRAPH/TABLE reference types and explicit nullability stay deferred.
    for feature in [FeatureId::GV60, FeatureId::GV61, FeatureId::GV90] {
        assert!(!SUPPORTED_FEATURES.contains(&feature), "{feature}");
        assert!(
            NOT_SUPPORTED_RATIONALE
                .iter()
                .any(|(unsupported, _)| *unsupported == feature),
            "{feature}"
        );
    }
}

#[test]
fn no_feature_is_both_supported_and_rationalized_unsupported() {
    for (feature, _) in NOT_SUPPORTED_RATIONALE {
        assert!(
            !SUPPORTED_FEATURES.contains(feature),
            "{feature} is in BOTH SUPPORTED_FEATURES and NOT_SUPPORTED_RATIONALE"
        );
    }
}

#[test]
fn quantifier_features_are_claimed_supported() {
    for feature in [
        FeatureId::G036,
        FeatureId::G037,
        FeatureId::G060,
        FeatureId::G061,
    ] {
        assert!(SUPPORTED_FEATURES.contains(&feature));
        assert!(
            !NOT_SUPPORTED_RATIONALE
                .iter()
                .any(|(unsupported, _)| *unsupported == feature)
        );
    }
}

#[test]
fn match_mode_features_are_claimed_supported() {
    // ISO 39075:2024 §16.4 CR1/CR2: G002 (DIFFERENT EDGES) and G003 (REPEATABLE
    // ELEMENTS) are claimed. They must be in SUPPORTED_FEATURES and must NOT
    // carry a NOT_SUPPORTED_RATIONALE entry.
    for feature in [FeatureId::G002, FeatureId::G003] {
        assert!(SUPPORTED_FEATURES.contains(&feature));
        assert!(
            !NOT_SUPPORTED_RATIONALE
                .iter()
                .any(|(unsupported, _)| *unsupported == feature)
        );
    }
}

#[test]
fn sql_drift_status_remaps_are_registered() {
    assert_eq!(GqlStatus::IMPLEMENTATION_DEFINED_ERROR.as_str(), "5GQL0");
    assert_eq!(GqlStatus::IMPLEMENTATION_DEFINED_ERROR.class(), *b"5G");
    assert!(gqlstatus_name("5GQL0").is_some());
    assert!(gqlstatus_name(&["XX", "500"].concat()).is_none());
    assert!(gqlstatus_name(&["220", "23"].concat()).is_none());

    assert_eq!(
        CoreError::StringTooLong { got: 2, max: 1 }.gqlstatus(),
        "22G03"
    );
    assert_eq!(
        GraphError::NodeNotFound { id: NodeId::new(1) }.gqlstatus(),
        "22G03"
    );
    assert_eq!(
        GraphError::Inconsistent {
            reason: "test".to_owned(),
        }
        .gqlstatus(),
        "5GQL0"
    );
    assert_eq!(
        PersistError::PrincipalTooLarge { len: 2, max: 1 }.gqlstatus(),
        "22G03"
    );
    assert_eq!(PersistError::MalformedSnapshotFilename.gqlstatus(), "5GQL0");
}
