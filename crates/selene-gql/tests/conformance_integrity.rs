//! BRIEF-125 conformance integrity regression coverage.

use selene_core::{CoreError, NodeId, feature_register::FeatureId, gqlstatus_name};
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
