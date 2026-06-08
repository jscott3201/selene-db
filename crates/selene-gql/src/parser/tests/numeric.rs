use crate::ast::{DecimalLiteralKind, FloatLiteralKind, Literal, SourceSpan, ValueExpr};

use super::only_item;

fn decimal(value: &str) -> rust_decimal::Decimal {
    value.parse().expect("decimal literal parses")
}

#[test]
fn parse_return_exact_decimal() {
    let item = only_item("RETURN 1.5");
    assert_eq!(
        item.expr,
        ValueExpr::Literal(Literal::Decimal(
            decimal("1.5"),
            SourceSpan::new(7, 3),
            DecimalLiteralKind::CommonWithoutSuffix,
        ))
    );
}

#[test]
fn parse_return_exact_decimal_forms() {
    for (source, value, len, kind) in [
        (
            "RETURN 1.5M",
            "1.5",
            4,
            DecimalLiteralKind::CommonOrIntegerWithSuffix,
        ),
        (
            "RETURN 1.5m",
            "1.5",
            4,
            DecimalLiteralKind::CommonOrIntegerWithSuffix,
        ),
        (
            "RETURN .5",
            "0.5",
            2,
            DecimalLiteralKind::CommonWithoutSuffix,
        ),
        ("RETURN 1.", "1", 2, DecimalLiteralKind::CommonWithoutSuffix),
        (
            "RETURN 1e2M",
            "100",
            4,
            DecimalLiteralKind::ScientificWithSuffix,
        ),
        (
            "RETURN 1.5e2m",
            "150",
            6,
            DecimalLiteralKind::ScientificWithSuffix,
        ),
    ] {
        assert_eq!(
            only_item(source).expr,
            ValueExpr::Literal(Literal::Decimal(
                decimal(value),
                SourceSpan::new(7, len),
                kind,
            )),
            "{source}",
        );
    }
}

#[test]
fn parse_return_float_forms() {
    for (source, kind) in [
        (
            "RETURN 1.5f",
            FloatLiteralKind::CommonOrIntegerWithFloatSuffix,
        ),
        (
            "RETURN 1.5F",
            FloatLiteralKind::CommonOrIntegerWithFloatSuffix,
        ),
        (
            "RETURN 1.5d",
            FloatLiteralKind::CommonOrIntegerWithDoubleSuffix,
        ),
        (
            "RETURN 1.5D",
            FloatLiteralKind::CommonOrIntegerWithDoubleSuffix,
        ),
    ] {
        assert_eq!(
            only_item(source).expr,
            ValueExpr::Literal(Literal::Float(1.5, SourceSpan::new(7, 4), kind)),
            "{source}",
        );
    }
    for (source, value, len, kind) in [
        (
            "RETURN 1e2",
            100.0,
            3,
            FloatLiteralKind::ScientificWithoutSuffix,
        ),
        (
            "RETURN 1e2f",
            100.0,
            4,
            FloatLiteralKind::ScientificWithFloatSuffix,
        ),
        (
            "RETURN 1e2D",
            100.0,
            4,
            FloatLiteralKind::ScientificWithDoubleSuffix,
        ),
        (
            "RETURN 1.0e30D",
            1.0e30,
            7,
            FloatLiteralKind::ScientificWithDoubleSuffix,
        ),
        (
            "RETURN 1F",
            1.0,
            2,
            FloatLiteralKind::CommonOrIntegerWithFloatSuffix,
        ),
        (
            "RETURN .5D",
            0.5,
            3,
            FloatLiteralKind::CommonOrIntegerWithDoubleSuffix,
        ),
        (
            "RETURN 1.F",
            1.0,
            3,
            FloatLiteralKind::CommonOrIntegerWithFloatSuffix,
        ),
    ] {
        assert_eq!(
            only_item(source).expr,
            ValueExpr::Literal(Literal::Float(value, SourceSpan::new(7, len), kind)),
            "{source}",
        );
    }
}

#[test]
fn parse_return_signed_float() {
    assert_eq!(
        only_item("RETURN -1.5").expr,
        ValueExpr::Literal(Literal::Decimal(
            decimal("-1.5"),
            SourceSpan::new(7, 4),
            DecimalLiteralKind::CommonWithoutSuffix,
        ))
    );
    assert_eq!(
        only_item("RETURN +0.25").expr,
        ValueExpr::Literal(Literal::Decimal(
            decimal("0.25"),
            SourceSpan::new(7, 5),
            DecimalLiteralKind::CommonWithoutSuffix,
        ))
    );
    assert_eq!(
        only_item("RETURN -1.5D").expr,
        ValueExpr::Literal(Literal::Float(
            -1.5,
            SourceSpan::new(7, 5),
            FloatLiteralKind::CommonOrIntegerWithDoubleSuffix,
        ))
    );
}
