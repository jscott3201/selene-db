//! Deferred procedure-surface parser gates.

use selene_core::feature_register::FeatureId;
use selene_gql::{ParserError, parse};

#[test]
fn procedure_reference_arguments_report_unsupported_features() {
    for (source, expected) in [
        ("CALL pkg.fn(TABLE rows)", FeatureId::GP14),
        ("CALL pkg.fn(GRAPH g)", FeatureId::GP15),
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
fn scalar_functions_do_not_accept_procedure_reference_arguments() {
    let error = parse("RETURN json_array(TABLE rows)").expect_err("procedure-only arg rejected");
    assert_eq!(error.gqlstatus().as_str(), "42001");
    assert!(
        matches!(error, ParserError::SyntaxError { .. }),
        "expected syntax error, got {error:?}"
    );
}
