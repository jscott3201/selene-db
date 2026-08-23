use super::*;

#[test]
fn parse_return_integer() {
    let item = only_item("RETURN 1");
    assert_eq!(
        item.expr,
        ValueExpr::Literal(Literal::Integer(1, SourceSpan::new(7, 1)))
    );
    assert_eq!(item.span, SourceSpan::new(7, 1));
}

#[test]
fn integer_literal_decimal_digit_boundary_matches_il010() {
    let item = only_item("RETURN 9223372036854775807");
    assert_eq!(
        item.expr,
        ValueExpr::Literal(Literal::Integer(i64::MAX, SourceSpan::new(7, 19)))
    );

    let error = parse("RETURN 18446744073709551615")
        .expect_err("a 20-digit u64 maximum does not fit in i64");
    let ParserError::SyntaxError { hint, .. } = error else {
        panic!("expected an integer-literal syntax error")
    };
    assert_eq!(hint.as_deref(), Some("integer literals must fit in i64"));
}

#[test]
fn parse_return_string() {
    let item = only_item("RETURN 'hello'");
    let ValueExpr::Literal(Literal::String(value, span, kind)) = &item.expr else {
        panic!("expected string literal");
    };
    assert_eq!(value.as_str(), "hello");
    assert_eq!(*span, SourceSpan::new(7, 7));
    assert_eq!(*kind, CharacterStringLiteralKind::Escaped);
}

#[test]
fn parse_return_bool_and_null() {
    assert_eq!(
        only_item("RETURN true").expr,
        ValueExpr::Literal(Literal::Bool(true, SourceSpan::new(7, 4)))
    );
    assert_eq!(
        only_item("RETURN false").expr,
        ValueExpr::Literal(Literal::Bool(false, SourceSpan::new(7, 5)))
    );
    assert_eq!(
        only_item("RETURN null").expr,
        ValueExpr::Literal(Literal::Null(SourceSpan::new(7, 4)))
    );
}

#[test]
fn parse_return_unknown() {
    // ISO/IEC 39075:2024 §21.2 <boolean literal> ::= TRUE | FALSE | UNKNOWN.
    // UNKNOWN is the mandatory-conformance boolean unknown truth value; the
    // runtime models it as `Value::Null` (validated 3VL), so the parser
    // lowers the `unknown_lit` token to `Literal::Null`.
    assert_eq!(
        only_item("RETURN UNKNOWN").expr,
        ValueExpr::Literal(Literal::Null(SourceSpan::new(7, 7)))
    );
    // Case-insensitive per the `^"UNKNOWN"` grammar rule.
    assert_eq!(
        only_item("RETURN unknown").expr,
        ValueExpr::Literal(Literal::Null(SourceSpan::new(7, 7)))
    );
}

#[test]
fn parse_return_alias() {
    let item = only_item("RETURN 1 AS one");
    assert_eq!(optional_name(item.alias).as_deref(), Some("one"));
    assert_eq!(item.span, SourceSpan::new(7, 8));
}

#[test]
fn parse_return_multiple_items() {
    let statement = return_clause("RETURN 1, 2.5, 'x'");
    assert_eq!(statement.items.len(), 3);
    assert_eq!(statement.span, SourceSpan::new(0, 18));
    assert_eq!(statement.items[0].span, SourceSpan::new(7, 1));
    assert_eq!(statement.items[1].span, SourceSpan::new(10, 3));
    assert_eq!(statement.items[2].span, SourceSpan::new(15, 3));
}

#[test]
fn parse_statement_span_covers_input() {
    let statement = parse("RETURN 1").expect("parse succeeds");
    assert_eq!(statement.span(), SourceSpan::new(0, 8));
}

#[test]
fn parse_return_signed_integer() {
    assert_eq!(
        only_item("RETURN -1").expr,
        ValueExpr::Literal(Literal::Integer(-1, SourceSpan::new(7, 2)))
    );
    assert_eq!(
        only_item("RETURN +42").expr,
        ValueExpr::Literal(Literal::Integer(42, SourceSpan::new(7, 3)))
    );
}

#[test]
fn parse_return_alias_quoted() {
    let item = only_item("RETURN 1 AS \"my name\"");
    assert_eq!(optional_name(item.alias).as_deref(), Some("my name"));
}

#[test]
fn parse_return_alias_quoted_doubled_quote() {
    let item = only_item("RETURN 1 AS \"a\"\"b\"");
    assert_eq!(optional_name(item.alias).as_deref(), Some("a\"b"));
}

#[test]
fn parse_return_alias_backtick() {
    let item = only_item("RETURN 1 AS `my name`");
    assert_eq!(optional_name(item.alias).as_deref(), Some("my name"));
}

#[test]
fn parse_return_alias_backtick_doubled_backtick() {
    let item = only_item("RETURN 1 AS `a``b`");
    assert_eq!(optional_name(item.alias).as_deref(), Some("a`b"));
}

#[test]
fn parse_temporal_literal() {
    let item = only_item("RETURN DATE '2020-01-01'");
    let ValueExpr::Literal(Literal::Date(value, span, kind)) = item.expr else {
        panic!("expected DATE literal");
    };
    assert_eq!(value.to_string(), "2020-01-01");
    assert_eq!(span, SourceSpan::new(7, 17));
    assert_eq!(kind, CharacterStringLiteralKind::Escaped);
}
