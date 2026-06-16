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

#[test]
fn session_set_value_expression_initializers_report_unsupported_features() {
    for (source, expected) in [
        (
            "SESSION SET VALUE $p = VALUE { MATCH (n) RETURN count(n) }",
            FeatureId::GS11,
        ),
        ("SESSION SET VALUE $p = 41 + 1", FeatureId::GS14),
        ("SESSION SET VALUE $p = ($base)", FeatureId::GS14),
        (
            "SESSION SET VALUE $p = VALUE { MATCH (n) RETURN count(n) } + 1",
            FeatureId::GS14,
        ),
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
fn session_set_value_specs_still_parse() {
    parse("SESSION SET VALUE $p = 42").expect("literal value spec parses");
    parse("SESSION SET VALUE $p = $base").expect("parameter value spec parses");
    parse("SESSION SET VALUE $p = $base :: INT64").expect("typed parameter value spec parses");
}
