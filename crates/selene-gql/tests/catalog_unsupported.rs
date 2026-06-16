//! Catalog-management syntax that is parsed but unsupported by this engine.

use selene_core::feature_register::FeatureId;
use selene_gql::{ParserError, parse};

#[test]
fn create_schema_is_rejected_before_planning() {
    for source in [
        "CREATE SCHEMA /myschema",
        "CREATE SCHEMA /foo/myschema",
        "CREATE SCHEMA IF NOT EXISTS /foo",
        "CREATE SCHEMA /foo NEXT CREATE SCHEMA /fee",
    ] {
        let error = parse(source).expect_err(source);
        assert_eq!(error.gqlstatus().as_str(), "42N01");
        let ParserError::UnsupportedFeature { feature_id, .. } = error else {
            panic!("expected UnsupportedFeature for {source}");
        };
        assert_eq!(feature_id, FeatureId::GC02, "{source}");
    }
}
