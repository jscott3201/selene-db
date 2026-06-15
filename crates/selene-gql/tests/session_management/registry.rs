use super::*;

#[test]
fn implemented_session_features_are_supported() {
    for id in [
        FeatureId::GS03,
        FeatureId::GS04,
        FeatureId::GS07,
        FeatureId::GS08,
        FeatureId::GS15,
        FeatureId::GS16,
    ] {
        assert!(SUPPORTED_FEATURES.contains(&id), "{id} must be supported");
    }
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
        assert!(
            !SUPPORTED_FEATURES.contains(&id),
            "{id} must not be claimed as supported"
        );
        assert!(
            NOT_SUPPORTED_RATIONALE
                .iter()
                .any(|(feature, _)| *feature == id),
            "{id} must carry a deferral rationale"
        );
    }
}

#[test]
fn session_defaults_are_registered_in_annex_b() {
    for id in ["ID048", "ID049"] {
        assert!(
            ANNEX_B_REGISTER
                .iter()
                .any(|(annex, _)| annex.as_str() == id),
            "{id} must be registered in Annex B"
        );
    }
}
