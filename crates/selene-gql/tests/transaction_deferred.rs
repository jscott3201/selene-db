//! Deferred transaction-control parser gates.

use selene_core::feature_register::FeatureId;
use selene_gql::{ParserError, Statement, parse};

#[test]
fn deferred_transaction_forms_report_unsupported_features() {
    for (source, expected) in [
        ("START TRANSACTION CREATE GRAPH demo", FeatureId::GP18),
        ("START TRANSACTION ON GRAPH demo", FeatureId::GT03),
    ] {
        let error = parse(source).expect_err(source);
        assert_eq!(error.gqlstatus().as_str(), "42N01");
        let ParserError::UnsupportedFeature { feature_id, .. } = error else {
            panic!("expected UnsupportedFeature for {source}");
        };
        assert_eq!(feature_id, expected, "{source}");
    }
}

#[test]
fn bare_start_transaction_still_parses() {
    let statement = parse("START TRANSACTION").expect("bare START TRANSACTION parses");
    assert!(matches!(statement, Statement::StartTransaction { .. }));
}
