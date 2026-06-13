//! Conformance coverage for ISO approximate numeric type-name synonyms.

use selene_core::feature_register::FeatureId;
use selene_gql::{
    ast::{format_read_statement, structurally_eq},
    feature_walk, parse,
};

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
    ] {
        let parsed = parse(source).expect(source);
        let formatted = format_read_statement(&parsed).expect("read statement formats");
        assert_eq!(formatted, expected);
        let reparsed = parse(&formatted).expect("formatted source parses");
        assert!(structurally_eq(&parsed, &reparsed));
    }
}
