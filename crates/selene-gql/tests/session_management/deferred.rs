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
fn selected_catalog_characteristics_parse_transportably() {
    assert!(matches!(
        parse("SESSION SET SCHEMA /memory").unwrap(),
        Statement::SessionSetGraph {
            target: SessionSetGraphTarget::SchemaReference(_),
            ..
        }
    ));
    assert!(matches!(
        parse("SESSION SET GRAPH /memory/main").unwrap(),
        Statement::SessionSetGraph {
            target: SessionSetGraphTarget::CatalogReference(_),
            ..
        }
    ));
    assert!(parse("SESSION SET SCHEMA memory").is_err());
    for source in [
        "SESSION RESET SCHEMA",
        "SESSION RESET GRAPH",
        "SESSION RESET PROPERTY GRAPH",
    ] {
        parse(source).unwrap_or_else(|error| panic!("{source}: {error}"));
    }
}

// ---------------------------------------------------------------------------
// feature_register hygiene (selene-core runtime/flagger inventory)
// ---------------------------------------------------------------------------
