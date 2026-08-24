//! Deferred transaction-control parser gates.

use selene_gql::{ParserError, Statement, parse};
use selene_profile::FeatureId;

#[test]
fn deferred_transaction_forms_report_unsupported_features() {
    for (source, expected) in [
        ("START TRANSACTION CREATE GRAPH demo ANY", FeatureId::GP18),
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

#[test]
fn transaction_keywords_require_boundaries() {
    for source in [
        "STARTTRANSACTION",
        "STARTx TRANSACTION",
        "START TRANSACTIONx",
        "START TRANSACTIONCREATE GRAPH demo ANY",
        "START TRANSACTION ONGRAPH demo",
        "START TRANSACTION ON GRAPHx",
        "COMMITx",
        "ROLLBACKx",
    ] {
        assert!(
            parse(source).is_err(),
            "{source} must reject keyword prefix"
        );
    }
}
