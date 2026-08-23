//! Flagger coverage for ISO optional features on ORDER BY sort keys.

use std::collections::BTreeSet;

use selene_core::feature_register::{FeatureId, SUPPORTED_FEATURES};
use selene_gql::{feature_walk, parse};

fn observed_features(source: &str) -> BTreeSet<FeatureId> {
    feature_walk(&parse(source).expect(source))
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect()
}

#[test]
fn sort_key_optional_features_are_runtime_supported() {
    for feature in [FeatureId::GQ14, FeatureId::GQ16, FeatureId::GF20] {
        assert!(
            SUPPORTED_FEATURES.contains(&feature),
            "{feature} must be runtime-supported"
        );
    }
}

#[test]
fn sort_key_feature_stamps_follow_sort_key_shape() {
    let bare_alias = observed_features("FOR x IN [1, 2] RETURN x ORDER BY x");
    assert!(bare_alias.contains(&FeatureId::GA07));
    assert!(!bare_alias.contains(&FeatureId::GQ14));
    assert!(!bare_alias.contains(&FeatureId::GQ16));
    assert!(!bare_alias.contains(&FeatureId::GF20));

    let complex_alias = observed_features("FOR x IN [1, 2] RETURN x AS x ORDER BY x + 1");
    assert!(complex_alias.contains(&FeatureId::GA07));
    assert!(complex_alias.contains(&FeatureId::GQ14));
    assert!(!complex_alias.contains(&FeatureId::GQ16));

    let pre_projection = observed_features("MATCH (n) RETURN n.name AS who ORDER BY n.age DESC");
    assert!(pre_projection.contains(&FeatureId::GA07));
    assert!(pre_projection.contains(&FeatureId::GQ14));
    assert!(pre_projection.contains(&FeatureId::GQ16));

    let aggregate = observed_features(
        "FOR x IN [1, 2] RETURN x AS x, count(*) AS c GROUP BY x ORDER BY count(*)",
    );
    assert!(aggregate.contains(&FeatureId::GF20));
}
