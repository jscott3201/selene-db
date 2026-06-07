use super::*;

#[test]
fn radix_integer_literals_parse_to_i64() {
    for (source, value, span, kind) in [
        (
            "RETURN 0xCAFE",
            51_966,
            SourceSpan::new(7, 6),
            IntegerLiteralKind::Hexadecimal,
        ),
        (
            "RETURN 0o777",
            511,
            SourceSpan::new(7, 5),
            IntegerLiteralKind::Octal,
        ),
        (
            "RETURN 0b1010",
            10,
            SourceSpan::new(7, 6),
            IntegerLiteralKind::Binary,
        ),
        (
            "RETURN 0x_1F",
            31,
            SourceSpan::new(7, 5),
            IntegerLiteralKind::Hexadecimal,
        ),
        (
            "RETURN -0x1A",
            -26,
            SourceSpan::new(7, 5),
            IntegerLiteralKind::Hexadecimal,
        ),
    ] {
        assert_eq!(
            only_item(source).expr,
            ValueExpr::Literal(Literal::RadixInteger(value, span, kind)),
            "{source}"
        );
    }
}

#[test]
fn malformed_radix_integer_literals_are_syntax_errors() {
    for source in [
        "RETURN 0x",
        "RETURN 0x_",
        "RETURN 0x1__2",
        "RETURN 0o8",
        "RETURN 0b102",
        "RETURN 0x8000000000000000",
    ] {
        let err = parse(source).expect_err("malformed radix literal should be rejected");
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "expected SyntaxError for {source:?}, got {err:?}"
        );
        assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR);
    }
}

#[test]
fn unsigned_suffix_literal_is_syntax_error() {
    let err = parse("RETURN 42u").expect_err("unsigned suffix should be rejected");
    assert!(matches!(err, ParserError::SyntaxError { .. }));
    assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR);
}
