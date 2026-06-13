//! ISO list value type spelling coverage.

use selene_gql::{
    GqlType, IsCheckKind, ParserError, PipelineStatement, Statement, ValueExpr,
    ast::{format_read_statement, structurally_eq},
    parse,
};

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
fn group_and_max_cardinality_list_type_forms_remain_deferred() {
    for source in [
        "RETURN NULL IS TYPED GROUP LIST<NODE> AS ok",
        "RETURN NULL IS TYPED GROUP ARRAY<EDGE> AS ok",
        "RETURN NULL IS TYPED LIST<STRING>[10] AS ok",
        "RETURN NULL IS TYPED STRING ARRAY[10] AS ok",
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
