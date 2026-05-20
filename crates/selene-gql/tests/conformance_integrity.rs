//! BRIEF-125 conformance integrity regression coverage.

use selene_core::{CoreError, NodeId, feature_register::FeatureId, gqlstatus_name};
use selene_gql::{GqlStatus, ParserError, parse};
use selene_graph::GraphError;
use selene_persist::PersistError;

#[test]
fn removed_pattern_alpha_surfaces_emit_42n01() {
    for (source, expected) in [
        ("MATCH ALL (n) RETURN n", FeatureId::G015),
        ("MATCH ANY (n) RETURN n", FeatureId::G016),
        ("MATCH ALL SHORTEST (a)-[:K]->(b) RETURN b", FeatureId::G017),
        ("MATCH ANY SHORTEST (a)-[:K]->(b) RETURN b", FeatureId::G018),
        ("CREATE GRAPH demo", FeatureId::GC04),
    ] {
        let error = parse(source).expect_err(source);
        assert_eq!(error.gqlstatus().as_str(), "42N01");
        let ParserError::UnsupportedFeature { feature_id, .. } = error else {
            panic!("expected UnsupportedFeature for {source:?}");
        };
        assert_eq!(feature_id, expected);
    }
}

#[test]
fn sql_drift_status_remaps_are_registered() {
    assert_eq!(GqlStatus::IMPLEMENTATION_DEFINED_ERROR.as_str(), "5GQL0");
    assert_eq!(GqlStatus::IMPLEMENTATION_DEFINED_ERROR.class(), *b"5G");
    assert!(gqlstatus_name("5GQL0").is_some());
    assert!(gqlstatus_name("XX500").is_none());
    assert!(gqlstatus_name("22023").is_none());

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
    assert_eq!(
        PersistError::MalformedSnapshotFilename.gqlstatus(),
        "5GQL0"
    );
}
