//! ISO list value type spelling coverage.

use selene_core::{GraphId, Value, db_string};
use selene_gql::{
    EmptyProcedureRegistry, ExecutorError, GqlType, IsCheckKind, ParserError, PipelineStatement,
    Session, Statement, ValueExpr,
    ast::{format_read_statement, structurally_eq},
    parse,
};
use selene_graph::SharedGraph;

#[test]
fn array_and_postfix_list_type_forms_parse_to_canonical_ast() {
    for (source, expected) in [
        (
            "RETURN NULL IS TYPED ARRAY<STRING> AS ok",
            GqlType::List(Box::new(GqlType::String)),
        ),
        (
            "RETURN NULL IS TYPED STRING ARRAY AS ok",
            GqlType::List(Box::new(GqlType::String)),
        ),
        (
            "RETURN NULL IS TYPED INTEGER LIST AS ok",
            GqlType::List(Box::new(GqlType::Integer)),
        ),
        (
            "RETURN NULL IS TYPED INTEGER LIST ARRAY AS ok",
            GqlType::List(Box::new(GqlType::List(Box::new(GqlType::Integer)))),
        ),
        (
            "RETURN NULL IS TYPED LIST<STRING>[10] AS ok",
            GqlType::BoundedList {
                element_type: Box::new(GqlType::String),
                max_len: 10,
            },
        ),
        (
            "RETURN NULL IS TYPED STRING ARRAY[0xA] AS ok",
            GqlType::BoundedList {
                element_type: Box::new(GqlType::String),
                max_len: 10,
            },
        ),
    ] {
        assert_eq!(typed_type(source), expected, "{source}");
    }
}

#[test]
fn postfix_list_type_forms_bind_element_and_outer_nullability() {
    assert_eq!(
        typed_type("RETURN NULL IS TYPED INTEGER NOT NULL ARRAY AS ok"),
        GqlType::List(Box::new(GqlType::NotNull(Box::new(GqlType::Integer))))
    );
    assert_eq!(
        typed_type("RETURN NULL IS TYPED INTEGER ARRAY NOT NULL AS ok"),
        GqlType::NotNull(Box::new(GqlType::List(Box::new(GqlType::Integer))))
    );
    assert_eq!(
        typed_type("RETURN NULL IS TYPED INTEGER NOT NULL ARRAY NOT NULL AS ok"),
        GqlType::NotNull(Box::new(GqlType::List(Box::new(GqlType::NotNull(
            Box::new(GqlType::Integer)
        )))))
    );
}

#[test]
fn list_type_forms_format_to_canonical_prefix_list() {
    for (source, expected) in [
        (
            "RETURN NULL IS TYPED ARRAY<STRING> AS ok",
            "RETURN null IS TYPED LIST<STRING> AS ok",
        ),
        (
            "RETURN NULL IS TYPED INTEGER ARRAY AS ok",
            "RETURN null IS TYPED LIST<INTEGER> AS ok",
        ),
        (
            "RETURN NULL IS TYPED INTEGER NOT NULL ARRAY AS ok",
            "RETURN null IS TYPED LIST<INTEGER NOT NULL> AS ok",
        ),
        (
            "RETURN NULL IS TYPED INTEGER ARRAY NOT NULL AS ok",
            "RETURN null IS TYPED LIST<INTEGER> NOT NULL AS ok",
        ),
        (
            "RETURN NULL IS TYPED INTEGER ARRAY[10] AS ok",
            "RETURN null IS TYPED LIST<INTEGER>[10] AS ok",
        ),
        (
            "RETURN NULL IS TYPED LIST<STRING>[10] NOT NULL AS ok",
            "RETURN null IS TYPED LIST<STRING>[10] NOT NULL AS ok",
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
fn bounded_list_type_predicates_enforce_max_cardinality() {
    assert_eq!(
        first_value("RETURN [1, 2] IS TYPED LIST<INTEGER>[2] AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN [1, 2, 3] IS TYPED LIST<INTEGER>[2] AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN ['x'] IS TYPED INTEGER ARRAY[1] AS ok"),
        Value::Bool(false)
    );
}

#[test]
fn bounded_list_typed_parameters_enforce_max_cardinality() {
    let graph = SharedGraph::new(GraphId::new(16_605));
    let mut session = Session::new(&graph);
    let name = db_string("xs").expect("valid parameter name");
    session.bind_parameter(
        name.clone(),
        Value::List(vec![Value::Int(1), Value::Int(2)]),
    );
    let output = session
        .execute_source(
            "RETURN $xs :: LIST<INTEGER>[2] AS xs",
            &EmptyProcedureRegistry,
        )
        .expect("in-bound list parameter is accepted");
    assert_eq!(
        first_output_value(output),
        Value::List(vec![Value::Int(1), Value::Int(2)])
    );

    session.bind_parameter(
        name,
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
    );
    let err = session
        .execute_source(
            "RETURN $xs :: LIST<INTEGER>[2] AS xs",
            &EmptyProcedureRegistry,
        )
        .expect_err("oversized list parameter rejects");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name,
            ref expected,
            actual: "LIST",
            ..
        } if name.as_str() == "xs" && expected == "LIST<INTEGER>[2]"
    ));
}

#[test]
fn zero_max_cardinality_rejects_at_parse_time() {
    let err = parse("RETURN NULL IS TYPED LIST<STRING>[0] AS ok")
        .expect_err("zero max cardinality should reject");
    assert!(matches!(err, ParserError::SyntaxError { .. }), "{err:?}");
}

#[test]
fn group_and_bare_list_type_forms_remain_deferred() {
    for source in [
        "RETURN NULL IS TYPED GROUP LIST<NODE> AS ok",
        "RETURN NULL IS TYPED GROUP ARRAY<EDGE> AS ok",
        "RETURN NULL IS TYPED LIST AS ok",
    ] {
        let err = parse(source).expect_err("deferred list type form should reject");
        assert!(
            matches!(
                err,
                ParserError::SyntaxError { .. } | ParserError::NotImplemented { .. }
            ),
            "{source} should reject as syntax/deferred builder gap, got {err:?}"
        );
    }
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
    let graph = SharedGraph::new(GraphId::new(16_604));
    let mut session = Session::new(&graph);
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    first_output_value(output)
}

fn first_output_value(output: selene_gql::StatementOutput) -> Value {
    let table = match output {
        selene_gql::StatementOutput::Rows(table) => table,
        other => panic!("expected query output, got {other:?}"),
    };
    table
        .rows()
        .first()
        .and_then(|row| row.values().first())
        .cloned()
        .expect("one value")
}
