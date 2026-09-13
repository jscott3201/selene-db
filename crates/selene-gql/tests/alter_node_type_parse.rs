//! Focused parser/AST coverage for additive `ALTER NODE TYPE`.

use selene_gql::{
    DdlStatement, GqlStatus, ParserError, Statement, TypePropertyConstraint, ast::structurally_eq,
    parse, parse_many,
};

#[test]
fn alter_node_type_parses_properties_and_preserves_spans() {
    let source = "ALTER NODE TYPE :Person (active :: BOOLEAN DEFAULT true, note STRING)";
    let statement = parse(source).expect("ALTER NODE TYPE parses");
    let Statement::Ddl(DdlStatement::AlterNodeType {
        label,
        properties,
        span,
    }) = &statement
    else {
        panic!("expected ALTER NODE TYPE AST, got {statement:?}");
    };

    assert_eq!(label.as_str(), "Person");
    assert_eq!(properties.len(), 2);
    assert!(matches!(
        properties[0].constraints.as_slice(),
        [TypePropertyConstraint::Default(_, _)]
    ));
    assert_eq!(span.byte_offset, 0);
    assert_eq!(usize::try_from(span.byte_len).unwrap(), source.len());
    assert!(properties.iter().all(|property| property.span.byte_len > 0));

    let shifted = parse(&format!("  {source}")).expect("shifted statement parses");
    assert!(
        structurally_eq(&statement, &shifted),
        "structural equality must ignore ALTER NODE TYPE source spans"
    );
}

#[test]
fn alter_node_type_requires_at_least_one_property() {
    let error = parse("ALTER NODE TYPE :Person ()").expect_err("empty alteration rejects");
    assert_eq!(error.gqlstatus(), GqlStatus::SYNTAX_ERROR);
    assert!(matches!(
        error,
        ParserError::SyntaxError { message, .. }
            if message.contains("must declare at least one property")
    ));
}

#[test]
fn alter_node_type_rejects_unsupported_type_identity_and_validation_changes() {
    for source in [
        "ALTER NODE TYPE :Person => (note STRING)",
        "ALTER NODE TYPE :Person & :Employee => (note STRING)",
        "ALTER NODE TYPE :Person (note STRING) STRICT",
        "ALTER NODE TYPE :Person (note STRING) WARN",
    ] {
        let error = parse(source).expect_err("non-property ALTER shape must reject");
        assert_eq!(error.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
    }
}

#[test]
fn parse_many_rebases_alter_node_type_and_property_spans() {
    let source = "SHOW NODE TYPES; ALTER NODE TYPE :Person (active BOOLEAN DEFAULT true)";
    let statements = parse_many(source).expect("batch parses");
    let Statement::Ddl(DdlStatement::AlterNodeType {
        properties, span, ..
    }) = &statements[1]
    else {
        panic!("expected second statement to be ALTER NODE TYPE");
    };
    let expected = u32::try_from(source.find("ALTER NODE TYPE").unwrap()).unwrap();
    assert_eq!(span.byte_offset, expected);
    assert!(properties[0].span.byte_offset >= expected);
}
