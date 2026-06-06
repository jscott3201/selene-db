//! Byte-string literal coverage for the native BYTES value type.

use std::sync::Arc;

use selene_core::{GraphId, Value};
use selene_gql::{
    AnalyzedStatement, AnalyzedStatementKind, AnalyzedType, EmptyProcedureRegistry, GqlType,
    ParserError, PipelineStatement, Session, StatementOutput,
    ast::{format_read_statement, structurally_eq},
    parse,
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

fn bytes(values: &[u8]) -> Value {
    Value::Bytes(Arc::<[u8]>::from(values))
}

fn analyze_one(source: &str) -> AnalyzedStatement {
    let statement = parse(source).expect("test source parses");
    selene_gql::analyze(statement, &EmptyProcedureRegistry, None).expect("test source analyzes")
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
