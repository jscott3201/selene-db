//! Byte-string literal coverage for the native BYTES value type.

use std::sync::Arc;

use selene_core::{GraphId, Value, feature_register::FeatureId};
use selene_gql::{
    AnalyzedStatement, AnalyzedStatementKind, AnalyzedType, EmptyProcedureRegistry, GqlType,
    ImplDefinedCaps, ParserError, PipelineStatement, Session, StatementOutput,
    ast::{format_read_statement, structurally_eq},
    feature_walk, parse,
};
use selene_graph::SharedGraph;

fn first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(14_200));
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
    let graph = SharedGraph::new(GraphId::new(14_201));
    let mut session = Session::new(&graph);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

fn first_value_with_caps(source: &str, caps: ImplDefinedCaps) -> Value {
    let graph = SharedGraph::new(GraphId::new(14_203));
    let mut session = Session::new(&graph).with_impl_defined_caps(caps);
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    let StatementOutput::Rows(table) = output else {
        panic!("`{source}` produced non-row output");
    };
    table.rows()[0].values()[0].clone()
}

fn first_status_with_caps(source: &str, caps: ImplDefinedCaps) -> String {
    let graph = SharedGraph::new(GraphId::new(14_204));
    let mut session = Session::new(&graph).with_impl_defined_caps(caps);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

fn bytes(values: &[u8]) -> Value {
    Value::Bytes(Arc::<[u8]>::from(values))
}

fn analyze_one(source: &str) -> AnalyzedStatement {
    let statement = parse(source).expect("test source parses");
    selene_gql::analyze(statement, &EmptyProcedureRegistry, None).expect("test source analyzes")
}

fn assert_feature_recorded(source: &str, expected: FeatureId) {
    let statement = parse(source).expect(source);
    let observed = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();
    assert!(
        observed.contains(&expected),
        "{source} should record {expected:?}, observed {observed:?}"
    );
}

fn projection_type(analyzed: &AnalyzedStatement, name: &str) -> AnalyzedType {
    let AnalyzedStatementKind::Query(query) = &analyzed.statement else {
        panic!("expected query statement");
    };
    let item = query
        .statements
        .iter()
        .filter_map(|statement| match statement {
            PipelineStatement::Return(clause) => Some(&clause.items),
            _ => None,
        })
        .flatten()
        .find(|item| {
            item.alias
                .clone()
                .is_some_and(|alias| alias.as_str() == name)
        })
        .unwrap_or_else(|| panic!("projection {name} exists"));
    let id = analyzed
        .expr_ids
        .get(&item.expr)
        .unwrap_or_else(|| panic!("projection {name} has an ExprId"));
    analyzed.expr_types.get(id).clone()
}

#[test]
fn byte_string_literal_executes_to_bytes_value() {
    assert_eq!(
        first_value("RETURN X'00 ff A5' AS payload"),
        bytes(&[0x00, 0xff, 0xa5])
    );
    assert_eq!(first_value("RETURN x'0a' AS payload"), bytes(&[0x0a]));
    assert_eq!(first_value("RETURN X'' AS payload"), bytes(&[]));
}

#[test]
fn byte_string_literal_chunks_concatenate_across_newline_separator() {
    assert_eq!(
        first_value("RETURN X'CA'\n'FE' AS payload"),
        bytes(&[0xca, 0xfe])
    );
}

#[test]
fn byte_string_literal_infers_bytes_and_is_typed_bytes() {
    let analyzed = analyze_one("RETURN X'CAFE' AS payload");
    assert_eq!(
        projection_type(&analyzed, "payload"),
        AnalyzedType::Resolved(GqlType::Bytes)
    );
    assert_eq!(
        first_value("RETURN X'CAFE' IS TYPED BYTES AS ok"),
        Value::Bool(true)
    );
}

#[test]
fn byte_string_literal_identity_cast_returns_bytes() {
    assert_eq!(
        first_value("RETURN CAST(X'CAFE' AS BYTES) AS payload"),
        bytes(&[0xca, 0xfe])
    );
}

#[test]
fn byte_string_left_and_right_follow_iso_substring_rules() {
    assert_eq!(
        first_value("RETURN left(X'CAFE00', 2) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN right(X'CAFE00', 2) AS payload"),
        bytes(&[0xfe, 0x00])
    );
    assert_eq!(
        first_value("RETURN left(X'CAFE', 99) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN right(X'CAFE', 0) AS payload"),
        bytes(&[])
    );
    assert_eq!(
        first_value("RETURN left(X'CAFE00', CAST('2' AS INT128)) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN right(X'CAFE00', CAST('2' AS UINT128)) AS payload"),
        bytes(&[0xfe, 0x00])
    );
    assert_eq!(
        first_value("RETURN left(X'CAFE00', 2M) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN left(X'CAFE', null) AS payload"),
        Value::Null
    );
    assert_eq!(first_value("RETURN left(null, 1) AS payload"), Value::Null);
    assert_eq!(first_status("RETURN left(X'CAFE', -1) AS payload"), "22011");
    assert_eq!(
        first_status("RETURN left(X'CAFE', 1.5M) AS payload"),
        "22G03"
    );
}

#[test]
fn byte_string_trim_follows_iso_trim_rules() {
    assert_eq!(
        first_value("RETURN TRIM(X'20CA20') AS payload"),
        bytes(&[0xca])
    );
    assert_eq!(
        first_value("RETURN TRIM(BOTH X'00' FROM X'0000CAFE00') AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN TRIM(LEADING X'00' FROM X'0000CAFE00') AS payload"),
        bytes(&[0xca, 0xfe, 0x00])
    );
    assert_eq!(
        first_value("RETURN TRIM(TRAILING X'00' FROM X'0000CAFE00') AS payload"),
        bytes(&[0x00, 0x00, 0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN TRIM(BOTH FROM X'20CA20') AS payload"),
        bytes(&[0xca])
    );
    assert_eq!(
        first_value("RETURN TRIM(BOTH X'00' FROM null) AS payload"),
        Value::Null
    );
    assert_eq!(
        first_value("RETURN TRIM(BOTH null FROM X'0000') AS payload"),
        Value::Null
    );
    assert_eq!(
        first_status("RETURN TRIM(BOTH X'' FROM X'CA') AS payload"),
        "22027"
    );
    assert_eq!(
        first_status("RETURN TRIM(BOTH X'CAFE' FROM X'CA') AS payload"),
        "22027"
    );
    assert_eq!(
        first_status("RETURN TRIM(BOTH 'x' FROM X'78') AS payload"),
        "22G03"
    );
    assert_eq!(
        first_status("RETURN TRIM(BOTH X'78' FROM 'xx') AS payload"),
        "22G03"
    );
}

#[test]
fn byte_string_trim_infers_bytes_and_records_gf07() {
    assert_eq!(
        projection_type(
            &analyze_one("RETURN TRIM(BOTH X'00' FROM X'00CA00') AS payload"),
            "payload"
        ),
        AnalyzedType::Resolved(GqlType::Bytes)
    );
    assert_feature_recorded(
        "RETURN TRIM(BOTH X'00' FROM X'00CA00') AS payload",
        FeatureId::GF07,
    );
    assert_feature_recorded("RETURN TRIM(X'20CA20') AS payload", FeatureId::GF07);
}

#[test]
fn byte_string_concatenation_uses_byte_string_result_type() {
    assert_eq!(
        first_value("RETURN X'CA' || X'FE00' AS payload"),
        bytes(&[0xca, 0xfe, 0x00])
    );
    assert_eq!(
        first_value("RETURN X'' || X'CA' AS payload"),
        bytes(&[0xca])
    );
    assert_eq!(first_status("RETURN X'CA' || 'FE' AS payload"), "22G03");
}

#[test]
fn byte_string_concatenation_truncates_only_zero_byte_overflow() {
    let caps = ImplDefinedCaps::default().with_max_byte_string_length(3);
    assert_eq!(
        first_value_with_caps("RETURN X'CAFE' || X'0000' AS payload", caps),
        bytes(&[0xca, 0xfe, 0x00])
    );
}

#[test]
fn byte_string_concatenation_reports_right_truncation() {
    let caps = ImplDefinedCaps::default().with_max_byte_string_length(1);
    assert_eq!(
        first_status_with_caps("RETURN X'CA' || X'FE' AS payload", caps),
        "22001"
    );
}

#[test]
fn bare_byte_string_type_aliases_normalize_to_bytes() {
    assert_eq!(
        projection_type(
            &analyze_one("RETURN CAST(X'CAFE' AS BINARY) AS payload"),
            "payload"
        ),
        AnalyzedType::Resolved(GqlType::Bytes)
    );
    assert_eq!(
        first_value("RETURN X'CAFE' IS TYPED BINARY AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN CAST(X'CAFE' AS VARBINARY) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN X'CAFE' IS TYPED VARBINARY AS ok"),
        Value::Bool(true)
    );
}

#[test]
fn byte_string_literal_formatter_canonicalizes_uppercase_hex() {
    let parsed = parse("RETURN x'00 ff a5' AS payload").expect("source parses");
    let formatted = format_read_statement(&parsed).expect("formats");
    assert_eq!(formatted, "RETURN X'00FFA5' AS payload");
    let reparsed = parse(&formatted).expect("formatted source reparses");
    assert!(structurally_eq(&parsed, &reparsed));
}

#[test]
fn byte_string_literal_rejects_odd_hex_digit_count() {
    let err = parse("RETURN X'0' AS payload").expect_err("odd hex count is rejected");
    let ParserError::SyntaxError { message, .. } = err else {
        panic!("expected syntax error");
    };
    assert!(message.contains("odd number of hexadecimal digits"));
}

#[test]
fn byte_string_literal_rejects_plain_space_between_chunks() {
    parse("RETURN X'CA' 'FE' AS payload").expect_err("chunk separator requires a newline");
}

#[test]
fn byte_string_literal_rejects_non_hex_digit() {
    parse("RETURN X'0G' AS payload").expect_err("non-hex digit is rejected");
}

#[test]
fn non_identity_bytes_casts_are_invalid_type_combinations() {
    for source in [
        "RETURN CAST('00' AS BYTES) AS payload",
        "RETURN CAST(1 AS BYTES) AS payload",
        "RETURN CAST(true AS BYTES) AS payload",
        "RETURN CAST(X'CAFE' AS STRING) AS payload",
        "RETURN CAST(X'CAFE' AS INTEGER) AS payload",
        "RETURN CAST(X'CAFE' AS BOOLEAN) AS payload",
        "RETURN CAST([X'CAFE'] AS LIST<STRING>) AS payload",
    ] {
        assert_eq!(first_status(source), "22G03", "{source}");
    }
}
