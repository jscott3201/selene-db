//! ISO open dynamic union value type coverage.

use selene_core::{GraphId, Value, db_string};
use selene_gql::{
    EmptyProcedureRegistry, GqlType, IsCheckKind, PipelineStatement, Session, Statement,
    StatementOutput, ValueExpr,
    ast::{format_read_statement, structurally_eq},
    parse,
};
use selene_graph::SharedGraph;

#[test]
fn open_dynamic_union_type_forms_parse_to_ast() {
    for source in [
        "RETURN NULL IS TYPED ANY AS ok",
        "RETURN NULL IS TYPED ANY VALUE AS ok",
    ] {
        assert_eq!(typed_type(source), GqlType::Any, "{source}");
    }

    for source in [
        "RETURN NULL IS TYPED PROPERTY VALUE AS ok",
        "RETURN NULL IS TYPED ANY PROPERTY VALUE AS ok",
    ] {
        assert_eq!(typed_type(source), GqlType::AnyProperty, "{source}");
    }
}

#[test]
fn open_dynamic_union_type_forms_format_canonically() {
    for (source, expected) in [
        (
            "RETURN NULL IS TYPED ANY VALUE AS ok",
            "RETURN null IS TYPED ANY AS ok",
        ),
        (
            "RETURN NULL IS TYPED PROPERTY VALUE AS ok",
            "RETURN null IS TYPED ANY PROPERTY VALUE AS ok",
        ),
        (
            "RETURN NULL IS TYPED ANY PROPERTY VALUE NOT NULL AS ok",
            "RETURN null IS TYPED ANY PROPERTY VALUE NOT NULL AS ok",
        ),
    ] {
        let parsed = parse(source).unwrap_or_else(|error| panic!("{source} parses: {error:?}"));
        let formatted = format_read_statement(&parsed).expect("read-side AST formats");
        assert_eq!(formatted, expected, "{source}");
        let reparsed =
            parse(&formatted).unwrap_or_else(|error| panic!("{formatted} reparses: {error:?}"));
        assert!(
            structurally_eq(&parsed, &reparsed),
            "{source} should round-trip through {formatted}"
        );
    }
}

#[test]
fn open_dynamic_union_predicates_enforce_membership() {
    assert_eq!(
        first_value("RETURN 1 IS TYPED ANY AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN NULL IS TYPED ANY NOT NULL AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN {a: 1} IS TYPED PROPERTY VALUE AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN NULL IS TYPED ANY PROPERTY VALUE NOT NULL AS ok"),
        Value::Bool(false)
    );
}

#[test]
fn open_dynamic_union_casts_are_identity_membership_checks() {
    assert_eq!(first_value("RETURN CAST(1 AS ANY) AS v"), Value::Int(1));
    let graph = SharedGraph::new(GraphId::new(16_608));
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("xs").expect("valid parameter name"),
        Value::List(vec![
            Value::Int(1),
            Value::String(db_string("x").expect("valid string")),
            Value::Bool(true),
        ]),
    );
    let output = session
        .execute_source("RETURN CAST($xs AS LIST) AS v", &EmptyProcedureRegistry)
        .expect("mixed list parameter casts through LIST<ANY>");
    assert_eq!(
        first_output_value(output),
        Value::List(vec![
            Value::Int(1),
            Value::String(db_string("x").expect("db string")),
            Value::Bool(true),
        ])
    );
    assert_eq!(
        first_value("RETURN CAST({a: 1} AS ANY PROPERTY VALUE) AS v"),
        first_value("RETURN {a: 1} AS v")
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
    let graph = SharedGraph::new(GraphId::new(16_606));
    let mut session = Session::new(&graph);
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    first_output_value(output)
}

fn first_output_value(output: StatementOutput) -> Value {
    let StatementOutput::Rows(table) = output else {
        panic!("statement should produce rows");
    };
    table
        .rows()
        .first()
        .and_then(|row| row.values().first())
        .cloned()
        .expect("one value")
}
