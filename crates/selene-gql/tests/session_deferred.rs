//! Deferred session-management parser gates.

use selene_core::feature_register::FeatureId;
use selene_gql::{ParserError, parse};

#[test]
fn session_set_binding_table_reports_unsupported_features() {
    for (source, expected) in [
        ("SESSION SET $t BINDING TABLE bindings", FeatureId::GS02),
        (
            "SESSION SET $t BINDING TABLE VALUE { MATCH (n) RETURN n }",
            FeatureId::GS10,
        ),
        ("SESSION SET $t BINDING TABLE $other", FeatureId::GS13),
    ] {
        let error = parse(source).expect_err(source);
        assert_eq!(error.gqlstatus().as_str(), "42N01");
        let ParserError::UnsupportedFeature { feature_id, .. } = error else {
            panic!("expected UnsupportedFeature for {source}");
        };
        assert_eq!(feature_id, expected, "{source}");
    }
}
