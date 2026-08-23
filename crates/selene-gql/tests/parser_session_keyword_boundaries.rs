//! Session-management keyword-boundary regression coverage.

use selene_gql::{ParserError, parse};
use selene_profile::FeatureId;

fn assert_syntax_error(source: &str) {
    let error = parse(source).expect_err(source);
    assert!(
        matches!(error, ParserError::SyntaxError { .. }),
        "{source} must reject as syntax, got {error:?}"
    );
}

fn assert_unsupported(source: &str, expected: FeatureId) {
    let error = parse(source).expect_err(source);
    assert_eq!(error.gqlstatus().as_str(), "42N01", "{source}");
    let ParserError::UnsupportedFeature { feature_id, .. } = error else {
        panic!("expected UnsupportedFeature for {source}");
    };
    assert_eq!(feature_id, expected, "{source}");
}

#[test]
fn session_command_heads_require_boundaries() {
    for source in [
        "SESSIONSET VALUE $p = 1",
        "SESSIONx SET VALUE $p = 1",
        "SESSION SETVALUE $p = 1",
        "SESSION SETx VALUE $p = 1",
        "SESSIONRESET",
        "SESSION RESETx",
        "SESSIONCLOSE",
        "SESSION CLOSEx",
    ] {
        assert_syntax_error(source);
    }
}

#[test]
fn session_set_keywords_require_boundaries() {
    for source in [
        "SESSION SET VALUEIF NOT EXISTS $p = 1",
        "SESSION SET VALUE IFNOT EXISTS $p = 1",
        "SESSION SET VALUE IF NOTEXISTS $p = 1",
        "SESSION SET VALUE IF NOT EXISTSx $p = 1",
        "SESSION SET TIMEZONE '+00:00'",
        "SESSION SET TIMEx ZONE '+00:00'",
        "SESSION SET TIME ZONEx '+00:00'",
        "SESSION SET GRAPHCURRENT_GRAPH",
        "SESSION SET GRAPHx CURRENT_GRAPH",
        "SESSION SET GRAPH CURRENT_GRAPHx",
        "SESSION SET PROPERTYGRAPH CURRENT_PROPERTY_GRAPH",
        "SESSION SET PROPERTYx GRAPH CURRENT_PROPERTY_GRAPH",
        "SESSION SET PROPERTY GRAPHx CURRENT_PROPERTY_GRAPH",
        "SESSION SET PROPERTY GRAPH CURRENT_PROPERTY_GRAPHx",
        "SESSION SET $g GRAPHCURRENT_GRAPH",
        "SESSION SET $g PROPERTYGRAPH CURRENT_GRAPH",
        "SESSION SET $t BINDINGTABLE bindings",
        "SESSION SET $t BINDINGx TABLE bindings",
        "SESSION SET $t BINDING TABLEx bindings",
        "SESSION SET $t BINDING TABLEbindings",
    ] {
        assert_syntax_error(source);
    }
}

#[test]
fn session_reset_keywords_require_boundaries() {
    for source in [
        "SESSION RESETSCHEMA",
        "SESSION RESETGRAPH",
        "SESSION RESETPROPERTY GRAPH",
        "SESSION RESET PROPERTYGRAPH",
        "SESSION RESET PROPERTYx GRAPH",
        "SESSION RESET PROPERTY GRAPHx",
        "SESSION RESETTIME ZONE",
        "SESSION RESET TIMEZONE",
        "SESSION RESET TIMEx ZONE",
        "SESSION RESET TIME ZONEx",
        "SESSION RESETALL PARAMETERS",
        "SESSION RESET ALLPARAMETERS",
        "SESSION RESET ALLx PARAMETERS",
        "SESSION RESET PARAMETERSx",
        "SESSION RESET CHARACTERISTICSx",
        "SESSION RESETPARAMETER $p",
        "SESSION RESET PARAMETERx $p",
    ] {
        assert_syntax_error(source);
    }
}

#[test]
fn guarded_session_keywords_still_accept_implemented_iso_forms() {
    for source in [
        "SESSION CLOSE",
        "SESSION SET VALUE $p = 1",
        "SESSION SET VALUE IF NOT EXISTS $p = 1",
        "SESSION SET TIME ZONE '+00:00'",
        "SESSION SET GRAPH CURRENT_GRAPH",
        "SESSION SET PROPERTY GRAPH CURRENT_PROPERTY_GRAPH",
        "SESSION RESET",
        "SESSION RESET PARAMETERS",
        "SESSION RESET ALL PARAMETERS",
        "SESSION RESET CHARACTERISTICS",
        "SESSION RESET ALL CHARACTERISTICS",
        "SESSION RESET TIME ZONE",
        "SESSION RESET PARAMETER $p",
    ] {
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    }
}

#[test]
fn guarded_session_keywords_still_accept_unsupported_iso_forms() {
    assert_unsupported("SESSION SET $g GRAPH CURRENT_GRAPH", FeatureId::GS01);
    assert_unsupported(
        "SESSION SET $g PROPERTY GRAPH CURRENT_GRAPH",
        FeatureId::GS01,
    );
    assert_unsupported("SESSION SET $g GRAPH $other", FeatureId::GS12);
    assert_unsupported("SESSION SET $t BINDING TABLE bindings", FeatureId::GS02);
    assert_unsupported("SESSION SET $t BINDING TABLE $other", FeatureId::GS13);
    assert_unsupported("SESSION RESET SCHEMA", FeatureId::GS05);
    assert_unsupported("SESSION RESET GRAPH", FeatureId::GS06);
    assert_unsupported("SESSION RESET PROPERTY GRAPH", FeatureId::GS06);
}
