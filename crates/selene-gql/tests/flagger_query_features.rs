//! Flagger coverage for query-level ISO feature IDs.

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
fn otherwise_and_let_use_their_distinct_iso_feature_ids() {
    assert!(SUPPORTED_FEATURES.contains(&FeatureId::GQ02));
    assert!(SUPPORTED_FEATURES.contains(&FeatureId::GQ09));

    let otherwise = observed_features("RETURN 1 AS n OTHERWISE RETURN 2 AS n");
    assert!(otherwise.contains(&FeatureId::GQ02));
    assert!(!otherwise.contains(&FeatureId::GQ09));

    let let_statement = observed_features("LET x = 1 RETURN x");
    assert!(let_statement.contains(&FeatureId::GQ09));
    assert!(!let_statement.contains(&FeatureId::GQ02));
}
