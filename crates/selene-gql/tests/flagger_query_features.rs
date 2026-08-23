//! Flagger coverage for query-level ISO feature IDs.

use std::collections::BTreeSet;

use selene_gql::{feature_walk, parse};
use selene_profile::{CapabilityStatus, FeatureId, capability};

fn observed_features(source: &str) -> BTreeSet<FeatureId> {
    feature_walk(&parse(source).expect(source))
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect()
}

#[test]
fn otherwise_and_let_use_their_distinct_iso_feature_ids() {
    assert_eq!(
        capability(FeatureId::GQ02).unwrap().status,
        CapabilityStatus::Supported
    );
    assert_eq!(
        capability(FeatureId::GQ09).unwrap().status,
        CapabilityStatus::Supported
    );

    let otherwise = observed_features("RETURN 1 AS n OTHERWISE RETURN 2 AS n");
    assert!(otherwise.contains(&FeatureId::GQ02));
    assert!(!otherwise.contains(&FeatureId::GQ09));

    let let_statement = observed_features("LET x = 1 RETURN x");
    assert!(let_statement.contains(&FeatureId::GQ09));
    assert!(!let_statement.contains(&FeatureId::GQ02));
}

#[test]
fn typed_let_value_definition_records_target_type_features() {
    let observed = observed_features("LET VALUE x INT8 = 1 RETURN x");

    assert!(observed.contains(&FeatureId::GQ09));
    assert!(observed.contains(&FeatureId::GV02));
    assert!(observed.contains(&FeatureId::GV09));
}
