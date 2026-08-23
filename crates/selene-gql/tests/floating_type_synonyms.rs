//! Conformance coverage for ISO approximate numeric type-name synonyms.

use selene_gql::{
    ParserError,
    ast::{format_read_statement, structurally_eq},
    feature_walk, parse,
};
use selene_profile::FeatureId;

fn assert_syntax_error(source: &str) {
    let err = parse(source).expect_err(source);
    assert!(
        matches!(err, ParserError::SyntaxError { .. }),
        "expected syntax error for `{source}`, got {err:?}"
    );
}

#[test]
fn floating_type_synonyms_flag_gv23_and_precision_feature() {
    for (source, precision_feature) in [
        ("RETURN n IS TYPED REAL", FeatureId::GV21),
        ("RETURN n IS TYPED DOUBLE", FeatureId::GV24),
        ("RETURN n IS TYPED DOUBLE PRECISION", FeatureId::GV24),
    ] {
        let observed = feature_walk(&parse(source).expect(source))
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            observed.contains(&FeatureId::GV23),
            "{source} must flag GV23; observed {observed:?}"
        );
        assert!(
            observed.contains(&precision_feature),
            "{source} must flag {precision_feature:?}; observed {observed:?}"
        );
    }
}

#[test]
fn floating_type_synonym_read_formatter_preserves_synonym_family() {
    for (source, expected) in [
        ("RETURN n IS TYPED REAL", "RETURN n IS TYPED REAL"),
        ("RETURN n IS TYPED DOUBLE", "RETURN n IS TYPED DOUBLE"),
        (
            "RETURN n IS TYPED DOUBLE PRECISION",
            "RETURN n IS TYPED DOUBLE",
        ),
        (
            "RETURN n IS TYPED DOUBLE /* c */ PRECISION",
            "RETURN n IS TYPED DOUBLE",
        ),
    ] {
        let parsed = parse(source).expect(source);
        let formatted = format_read_statement(&parsed).expect("read statement formats");
        assert_eq!(formatted, expected);
        let reparsed = parse(&formatted).expect("formatted source parses");
        assert!(structurally_eq(&parsed, &reparsed));
    }
}

#[test]
fn floating_type_synonym_keywords_require_boundaries() {
    for source in [
        "RETURN n IS TYPED REALx",
        "RETURN n IS TYPED DOUBLEx",
        "RETURN n IS TYPED DOUBLEPRECISION",
        "RETURN n IS TYPED DOUBLEPRECISIONx",
        "RETURN n IS TYPED DOUBLE PRECISIONx",
    ] {
        assert_syntax_error(source);
    }
}

#[test]
fn unsupported_float_width_names_report_the_same_feature_ids() {
    for (source, expected_feature) in [
        ("RETURN n IS TYPED FLOAT16", FeatureId::GV20),
        ("RETURN n IS TYPED FLOAT128", FeatureId::GV25),
        ("RETURN n IS TYPED FLOAT256", FeatureId::GV26),
    ] {
        let err = parse(source).expect_err(source);
        let ParserError::UnsupportedFeature { feature_id, .. } = err else {
            panic!("expected unsupported feature for `{source}`, got {err:?}");
        };
        assert_eq!(feature_id, expected_feature);
    }
}
