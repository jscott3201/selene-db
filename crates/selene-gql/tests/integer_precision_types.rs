//! Conformance coverage for ISO specified integer precision type names.

use selene_core::{DbString, GraphId, Value};
use selene_gql::{
    EmptyProcedureRegistry, ParserError, Session, StatementOutput,
    ast::{format_read_statement, structurally_eq},
    feature_walk, parse,
};
use selene_graph::SharedGraph;
use selene_profile::FeatureId;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_724));
    let mut session = Session::new(&graph);
    first_value_in(&mut session, source)
}

fn first_value_in(session: &mut Session<'_>, source: &str) -> Value {
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    let StatementOutput::Rows(table) = output else {
        panic!("`{source}` produced non-row output");
    };
    table.rows()[0].values()[0].clone()
}

fn first_status(source: &str) -> String {
    let graph = SharedGraph::new(GraphId::new(13_725));
    let mut session = Session::new(&graph);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

fn bind_and_eval(value: Value, source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_726));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("p"), value);
    first_value_in(&mut session, source)
}

fn assert_syntax_error(source: &str) {
    let err = parse(source).expect_err(source);
    assert!(
        matches!(err, ParserError::SyntaxError { .. }),
        "expected syntax error for `{source}`, got {err:?}"
    );
}

#[test]
fn specified_integer_precision_formats_to_supported_normal_form() {
    for (source, expected) in [
        ("RETURN n IS TYPED INT(7)", "RETURN n IS TYPED INT8"),
        ("RETURN n IS TYPED INT(8)", "RETURN n IS TYPED INT16"),
        ("RETURN n IS TYPED INTEGER(31)", "RETURN n IS TYPED INT32"),
        (
            "RETURN n IS TYPED SIGNED INTEGER(32)",
            "RETURN n IS TYPED INT64",
        ),
        (
            "RETURN n IS TYPED SIGNED /* c */ INTEGER(32)",
            "RETURN n IS TYPED INT64",
        ),
        ("RETURN n IS TYPED INT(64)", "RETURN n IS TYPED INT128"),
        ("RETURN n IS TYPED INT(1_27)", "RETURN n IS TYPED INT128"),
        ("RETURN n IS TYPED UINT(8)", "RETURN n IS TYPED UINT8"),
        ("RETURN n IS TYPED UINT(9)", "RETURN n IS TYPED UINT16"),
        (
            "RETURN n IS TYPED UNSIGNED INTEGER(32)",
            "RETURN n IS TYPED UINT32",
        ),
        (
            "RETURN n IS TYPED UNSIGNED /* c */ INTEGER(32)",
            "RETURN n IS TYPED UINT32",
        ),
        ("RETURN n IS TYPED UINT(33)", "RETURN n IS TYPED UINT64"),
        ("RETURN n IS TYPED UINT(65)", "RETURN n IS TYPED UINT128"),
        ("RETURN n IS TYPED UINT(1_28)", "RETURN n IS TYPED UINT128"),
    ] {
        let parsed = parse(source).expect(source);
        let formatted = format_read_statement(&parsed).expect("read statement formats");
        assert_eq!(formatted, expected);
        let reparsed = parse(&formatted).expect("formatted source parses");
        assert!(structurally_eq(&parsed, &reparsed), "{source}");
    }
}

#[test]
fn specified_integer_precision_keywords_require_boundaries() {
    for source in [
        "RETURN n IS TYPED SIGNEDINTEGER(32)",
        "RETURN n IS TYPED UNSIGNEDINTEGER(32)",
        "RETURN n IS TYPED INTEGERx(31)",
        "RETURN n IS TYPED INTx(31)",
        "RETURN n IS TYPED UINTx(31)",
    ] {
        assert_syntax_error(source);
    }
}

#[test]
fn specified_integer_precision_records_gv09_and_normal_form_width() {
    for (source, expected_width_feature) in [
        ("RETURN n IS TYPED INT(7)", FeatureId::GV02),
        ("RETURN n IS TYPED INT(8)", FeatureId::GV04),
        ("RETURN n IS TYPED INTEGER(31)", FeatureId::GV07),
        ("RETURN n IS TYPED SIGNED INTEGER(64)", FeatureId::GV14),
        ("RETURN n IS TYPED UINT(8)", FeatureId::GV01),
        ("RETURN n IS TYPED UINT(9)", FeatureId::GV03),
        ("RETURN n IS TYPED UNSIGNED INTEGER(33)", FeatureId::GV11),
        ("RETURN n IS TYPED UINT(65)", FeatureId::GV13),
    ] {
        let observed = feature_walk(&parse(source).expect(source))
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            observed.contains(&FeatureId::GV09),
            "{source} must flag GV09; observed {observed:?}"
        );
        assert!(
            observed.contains(&expected_width_feature),
            "{source} must flag {expected_width_feature:?}; observed {observed:?}"
        );
    }
}

#[test]
fn signed_precision_types_use_selected_runtime_width() {
    for (source, expected) in [
        ("RETURN 127 IS TYPED INT(7) AS ok", true),
        ("RETURN 128 IS TYPED INT(7) AS ok", false),
        ("RETURN 128 IS TYPED INT(8) AS ok", true),
        ("RETURN -32769 IS TYPED INTEGER(15) AS ok", false),
        ("RETURN 2147483648 IS TYPED INTEGER(31) AS ok", false),
    ] {
        assert_eq!(first_value(source), Value::Bool(expected), "{source}");
    }

    assert_eq!(
        first_value("RETURN CAST(128 AS INT(8)) AS v"),
        Value::Int(128)
    );
    assert_eq!(first_status("RETURN CAST(128 AS INT(7)) AS v"), "22003");
}

#[test]
fn unsigned_precision_types_use_selected_runtime_width() {
    assert_eq!(
        bind_and_eval(Value::Uint(255), "RETURN $p IS TYPED UINT(8) AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        bind_and_eval(Value::Uint(256), "RETURN $p IS TYPED UINT(8) AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        bind_and_eval(Value::Uint(256), "RETURN $p IS TYPED UINT(9) AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN CAST(4294967295 AS UINT(32)) AS v"),
        Value::Uint(u64::from(u32::MAX))
    );
    assert_eq!(
        first_status("RETURN CAST(4294967296 AS UINT(32)) AS v"),
        "22003"
    );
}

#[test]
fn invalid_or_unsupported_precision_reports_honest_parse_errors() {
    for source in [
        "RETURN n IS TYPED INT(0)",
        "RETURN n IS TYPED UINT(0)",
        "RETURN n IS TYPED INT(7_)",
        "RETURN n IS TYPED UINT(1__2)",
    ] {
        assert_syntax_error(source);
    }

    for (source, expected_feature) in [
        ("RETURN n IS TYPED INT(128)", FeatureId::GV16),
        ("RETURN n IS TYPED SIGNED INTEGER(128)", FeatureId::GV16),
        ("RETURN n IS TYPED UINT(129)", FeatureId::GV15),
        ("RETURN n IS TYPED UNSIGNED INTEGER(129)", FeatureId::GV15),
    ] {
        let err = parse(source).expect_err(source);
        let ParserError::UnsupportedFeature { feature_id, .. } = err else {
            panic!("expected unsupported feature for `{source}`, got {err:?}");
        };
        assert_eq!(feature_id, expected_feature);
    }
}
