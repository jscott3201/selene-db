use super::*;

#[test]
fn deferred_set_graph_parameter_reports_unsupported_features() {
    for (source, expected) in [
        ("SESSION SET $g GRAPH CURRENT_GRAPH", FeatureId::GS01),
        (
            "SESSION SET $g PROPERTY GRAPH CURRENT_GRAPH",
            FeatureId::GS01,
        ),
        ("SESSION SET $g GRAPH $other", FeatureId::GS12),
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
fn keyword_first_graph_parameter_forms_still_fail_to_parse() {
    assert!(parse("SESSION SET GRAPH $g CURRENT_GRAPH").is_err());
    assert!(parse("SESSION SET PROPERTY GRAPH $g CURRENT_PROPERTY_GRAPH").is_err());
}

#[test]
fn deferred_set_schema_fails_to_parse() {
    // SESSION SET <name> SCHEMA (D1: no schema layer) is not in the grammar.
    assert!(parse("SESSION SET SCHEMA myschema").is_err());
}

#[test]
fn deferred_reset_schema_and_graph_report_unsupported_features() {
    for (source, expected) in [
        ("SESSION RESET SCHEMA", FeatureId::GS05),
        ("SESSION RESET GRAPH", FeatureId::GS06),
        ("SESSION RESET PROPERTY GRAPH", FeatureId::GS06),
    ] {
        let error = parse(source).expect_err(source);
        assert_eq!(error.gqlstatus().as_str(), "42N01");
        let ParserError::UnsupportedFeature { feature_id, .. } = error else {
            panic!("expected UnsupportedFeature for {source}");
        };
        assert_eq!(feature_id, expected, "{source}");
    }
}

// ---------------------------------------------------------------------------
// feature_register hygiene (selene-core runtime/flagger inventory)
// ---------------------------------------------------------------------------
