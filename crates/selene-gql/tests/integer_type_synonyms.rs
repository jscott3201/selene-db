//! Conformance coverage for ISO binary exact numeric type-name synonyms.

use selene_core::{GraphId, Value};
use selene_gql::{
    EmptyProcedureRegistry, ParserError, Session, StatementOutput,
    ast::{format_read_statement, structurally_eq},
    feature_walk, parse,
};
use selene_graph::SharedGraph;
use selene_profile::FeatureId;

fn first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_722));
    let mut session = Session::new(&graph);
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    let StatementOutput::Rows(table) = output else {
        panic!("`{source}` produced non-row output");
    };
    table.rows()[0].values()[0].clone()
}

fn first_status(source: &str) -> String {
    let graph = SharedGraph::new(GraphId::new(13_723));
    let mut session = Session::new(&graph);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

fn assert_syntax_error(source: &str) {
    let err = parse(source).expect_err(source);
    assert!(
        matches!(err, ParserError::SyntaxError { .. }),
        "expected syntax error for `{source}`, got {err:?}"
    );
}

#[test]
fn verbose_integer_type_names_format_to_canonical_names() {
    for (source, expected) in [
        ("RETURN n IS TYPED INTEGER8", "RETURN n IS TYPED INT8"),
        (
            "RETURN n IS TYPED SIGNED INTEGER16",
            "RETURN n IS TYPED INT16",
        ),
        (
            "RETURN n IS TYPED SMALL INTEGER",
            "RETURN n IS TYPED SMALLINT",
        ),
        (
            "RETURN n IS TYPED SIGNED BIG INTEGER",
            "RETURN n IS TYPED BIGINT",
        ),
        (
            "RETURN n IS TYPED UNSIGNED INTEGER8",
            "RETURN n IS TYPED UINT8",
        ),
        (
            "RETURN n IS TYPED UNSIGNED INTEGER32",
            "RETURN n IS TYPED UINT32",
        ),
        (
            "RETURN n IS TYPED UNSIGNED SMALL INTEGER",
            "RETURN n IS TYPED USMALLINT",
        ),
        (
            "RETURN n IS TYPED UNSIGNED INTEGER",
            "RETURN n IS TYPED UINT",
        ),
        (
            "RETURN n IS TYPED UNSIGNED BIG INTEGER",
            "RETURN n IS TYPED UBIGINT",
        ),
        (
            "RETURN n IS TYPED SIGNED /* c */ SMALL /* c */ INTEGER",
            "RETURN n IS TYPED SMALLINT",
        ),
        (
            "RETURN n IS TYPED SIGNED /* c */ INTEGER16",
            "RETURN n IS TYPED INT16",
        ),
        (
            "RETURN n IS TYPED UNSIGNED /* c */ BIG /* c */ INTEGER",
            "RETURN n IS TYPED UBIGINT",
        ),
        (
            "RETURN n IS TYPED UNSIGNED /* c */ INTEGER8",
            "RETURN n IS TYPED UINT8",
        ),
        (
            "RETURN n IS TYPED BIG /* c */ INTEGER",
            "RETURN n IS TYPED BIGINT",
        ),
    ] {
        let parsed = parse(source).expect(source);
        let formatted = format_read_statement(&parsed).expect("read statement formats");
        assert_eq!(formatted, expected);
        let reparsed = parse(&formatted).expect("formatted source parses");
        assert!(structurally_eq(&parsed, &reparsed), "{source}");
    }
}

#[test]
fn integer_type_keywords_require_boundaries() {
    for source in [
        "RETURN n IS TYPED BOOLEANx",
        "RETURN n IS TYPED BOOLx",
        "RETURN n IS TYPED SIGNEDSMALLINTEGER",
        "RETURN n IS TYPED SIGNEDBIGINTEGER",
        "RETURN n IS TYPED SIGNEDINTEGER16",
        "RETURN n IS TYPED SIGNEDINTEGER",
        "RETURN n IS TYPED SIGNEDINTEGER256",
        "RETURN n IS TYPED UNSIGNEDSMALLINTEGER",
        "RETURN n IS TYPED UNSIGNEDBIGINTEGER",
        "RETURN n IS TYPED UNSIGNEDINTEGER8",
        "RETURN n IS TYPED UNSIGNEDINTEGER",
        "RETURN n IS TYPED UNSIGNEDINTEGER256",
        "RETURN n IS TYPED BIGINTEGER",
        "RETURN n IS TYPED SMALLINTEGER",
        "RETURN n IS TYPED INTEGER8x",
        "RETURN n IS TYPED INT8x",
        "RETURN n IS TYPED UINT8x",
        "RETURN n IS TYPED BIGINTx",
        "RETURN n IS TYPED USMALLINTx",
    ] {
        assert_syntax_error(source);
    }
}

#[test]
fn unsigned_category_type_names_flag_their_own_features() {
    for (source, expected_feature) in [
        ("RETURN n IS TYPED USMALLINT", FeatureId::GV05),
        ("RETURN n IS TYPED UINT", FeatureId::GV08),
        ("RETURN n IS TYPED UBIGINT", FeatureId::GV10),
    ] {
        let observed = feature_walk(&parse(source).expect(source))
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            observed.contains(&expected_feature),
            "{source} must flag {expected_feature:?}; observed {observed:?}"
        );
    }
}

#[test]
fn verbose_integer_type_names_use_existing_runtime_widths() {
    for (source, expected) in [
        ("RETURN 127 IS TYPED INTEGER8 AS ok", true),
        ("RETURN 128 IS TYPED INTEGER8 AS ok", false),
        ("RETURN -32768 IS TYPED SIGNED INTEGER16 AS ok", true),
        ("RETURN 32768 IS TYPED SMALL INTEGER AS ok", false),
        ("RETURN CAST(127 AS SIGNED INTEGER8) = 127 AS ok", true),
        (
            "RETURN CAST(255 AS UNSIGNED INTEGER8) = CAST(255 AS UINT8) AS ok",
            true,
        ),
    ] {
        assert_eq!(first_value(source), Value::Bool(expected), "{source}");
    }
}

#[test]
fn unsigned_category_type_names_enforce_distinct_ranges() {
    assert_eq!(
        first_value("RETURN CAST(65535 AS USMALLINT) IS TYPED USMALLINT AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN CAST(65536 AS UINT) IS TYPED UINT AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN CAST(4294967296 AS UBIGINT) IS TYPED UBIGINT AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN CAST(65536 AS UINT) IS TYPED USMALLINT AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN CAST(4294967296 AS UBIGINT) IS TYPED UINT AS ok"),
        Value::Bool(false)
    );

    for source in [
        "RETURN CAST(65536 AS USMALLINT) AS v",
        "RETURN CAST(4294967296 AS UINT) AS v",
    ] {
        assert_eq!(first_status(source), "22003", "{source}");
    }
}

#[test]
fn verbose_unsupported_256_bit_names_report_the_same_feature_ids() {
    for (source, expected_feature) in [
        ("RETURN n IS TYPED INTEGER256", FeatureId::GV16),
        ("RETURN n IS TYPED SIGNED INTEGER256", FeatureId::GV16),
        (
            "RETURN n IS TYPED SIGNED /* c */ INTEGER256",
            FeatureId::GV16,
        ),
        ("RETURN n IS TYPED UNSIGNED INTEGER256", FeatureId::GV15),
        (
            "RETURN n IS TYPED UNSIGNED /* c */ INTEGER256",
            FeatureId::GV15,
        ),
    ] {
        let err = parse(source).expect_err(source);
        let ParserError::UnsupportedFeature { feature_id, .. } = err else {
            panic!("expected unsupported feature for `{source}`, got {err:?}");
        };
        assert_eq!(feature_id, expected_feature);
    }
}
