use super::*;

#[test]
fn implication_consistent_session_features_are_supported() {
    for id in [
        FeatureId::GS03,
        FeatureId::GS07,
        FeatureId::GS08,
        FeatureId::GS15,
        FeatureId::GS16,
    ] {
        assert!(supported(id), "{id} must be supported");
    }
}

#[test]
fn reset_all_is_admitted_independently_from_runtime_status() {
    let record = capability(FeatureId::GS04).expect("GS04 capability");
    assert_eq!(record.status, CapabilityStatus::Unsupported);
    assert_eq!(record.flagger_status, FlaggerStatus::Accepted);
    assert!(!record.non_support_rationale.is_empty());
    selene_gql::parse("SESSION RESET ALL CHARACTERISTICS").expect("GS04 syntax is admitted");
}

#[test]
fn deferred_session_features_have_d1_rationale() {
    for id in [
        FeatureId::GS01,
        FeatureId::GS02,
        FeatureId::GS05,
        FeatureId::GS06,
        FeatureId::GS10,
        FeatureId::GS11,
        FeatureId::GS12,
        FeatureId::GS13,
        FeatureId::GS14,
    ] {
        assert!(!supported(id), "{id} must not be runtime-supported");
        assert!(
            capability(id).is_some_and(|record| !record.non_support_rationale.is_empty()),
            "{id} must carry a deferral rationale"
        );
    }
}

#[test]
fn session_defaults_are_registered_in_annex_b() {
    for id in ["ID048", "ID049"] {
        assert!(
            annex_b_by_id(id).is_some(),
            "{id} must be registered in Annex B"
        );
    }
}
