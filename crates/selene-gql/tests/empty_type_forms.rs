//! ISO empty value type spelling coverage.

use selene_core::{GraphId, Value};
use selene_gql::{
    EmptyProcedureRegistry, GqlType, IsCheckKind, ParserError, PipelineStatement, Session,
    Statement, StatementOutput, ValueExpr,
    ast::{format_read_statement, structurally_eq},
    parse,
};
use selene_graph::SharedGraph;

#[test]
fn null_not_null_parses_to_empty_type_normal_form() {
    assert_eq!(
        typed_type("RETURN NULL IS TYPED NULL NOT NULL AS ok"),
        GqlType::Nothing
    );
    assert_eq!(
        typed_type("RETURN [] IS TYPED NULL NOT NULL ARRAY AS ok"),
        GqlType::List(Box::new(GqlType::Nothing))
    );
    assert_eq!(
        typed_type("RETURN [] IS TYPED NULL NOT NULL ARRAY NOT NULL AS ok"),
        GqlType::NotNull(Box::new(GqlType::List(Box::new(GqlType::Nothing))))
    );
}

#[test]
fn null_not_null_formats_to_nothing_normal_form() {
    let parsed = parse("RETURN NULL IS TYPED NULL NOT NULL AS ok").expect("source parses");
    let formatted = format_read_statement(&parsed).expect("read-side AST formats");
    assert_eq!(formatted, "RETURN null IS TYPED NOTHING AS ok");
    let reparsed = parse(&formatted).expect("formatted source reparses");
    assert!(structurally_eq(&parsed, &reparsed));
}

#[test]
fn redundant_nothing_not_null_is_rejected() {
    for source in [
        "RETURN NULL IS TYPED NOTHING NOT NULL AS ok",
        "RETURN [] IS TYPED NOTHING NOT NULL ARRAY AS ok",
        "RETURN [] IS TYPED LIST<NOTHING NOT NULL> AS ok",
    ] {
        let err = parse(source).expect_err("redundant NOTHING NOT NULL rejects");
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "{source} should reject as syntax, got {err:?}"
        );
    }
}

#[test]
fn empty_type_normal_form_keeps_runtime_membership() {
    assert_eq!(
        first_value("RETURN NULL IS TYPED NULL NOT NULL AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN 1 IS NOT TYPED NULL NOT NULL AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN [] IS TYPED NULL NOT NULL ARRAY AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN [NULL] IS TYPED NULL NOT NULL ARRAY AS ok"),
        Value::Bool(false)
    );
}

fn typed_type(source: &str) -> GqlType {
    let statement =
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    let Statement::Query(pipeline) = statement else {
        panic!("{source} should parse as a query");
    };
    let PipelineStatement::Return(return_clause) = &pipeline.statements[0] else {
        panic!("{source} should parse as RETURN");
    };
    let ValueExpr::IsCheck {
        kind: IsCheckKind::Typed(ty),
        ..
    } = &return_clause.items[0].expr
    else {
        panic!("{source} should parse as IS TYPED");
    };
    ty.clone()
}

fn first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_820));
    let mut session = Session::new(&graph);
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|error| panic!("{source} should execute: {error:?}"));
    let StatementOutput::Rows(table) = output else {
        panic!("{source} should produce rows");
    };
    assert_eq!(table.row_count(), 1);
    table.rows()[0].values()[0].clone()
}
