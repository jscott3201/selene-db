//! Pest-backed GQL parser entry points.

mod builders;

use pest::{Parser, error::InputLocation};

use crate::{
    ast::{SourceSpan, Statement},
    error::ParserError,
};

use self::pest_impl::{GqlParser, Rule};

mod pest_impl {
    #![allow(missing_docs)]

    #[derive(pest_derive::Parser)]
    #[grammar = "parser/grammar.pest"]
    pub(crate) struct GqlParser;
}

/// Parse one GQL program.
///
/// BRIEF-16 constructs AST nodes only for literal `RETURN` statements. The
/// grammar already recognizes the broader language surface so later briefs can
/// add builders without another grammar port.
///
/// # Errors
///
/// Returns [`ParserError::SyntaxError`] for parse failures and
/// [`ParserError::NotImplemented`] for grammar surfaces whose AST builders
/// intentionally land after BRIEF-16.
pub fn parse(source: &str) -> Result<Statement, ParserError> {
    let mut pairs =
        GqlParser::parse(Rule::gql_program, source).map_err(|error| pest_error(source, error))?;
    let program_pair = pairs.next().ok_or_else(ParserError::empty_program)?;
    builders::build_statement(program_pair)
}

fn pest_error(source: &str, error: pest::error::Error<Rule>) -> ParserError {
    let span = match error.location {
        InputLocation::Pos(offset) => point_span(offset),
        InputLocation::Span((start, end)) => SourceSpan::new(to_u32(start), to_u32(end - start)),
    };
    let message = if source.is_empty() {
        "empty GQL program".to_owned()
    } else {
        error.variant.message().to_string()
    };
    ParserError::syntax(
        message,
        span,
        Some("check GQL syntax near the highlighted span".into()),
    )
}

fn point_span(offset: usize) -> SourceSpan {
    SourceSpan::new(to_u32(offset), 0)
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use selene_core::intern;

    use super::*;
    use crate::ast::{Literal, ValueExpr};
    use crate::error::GqlStatus;

    fn only_item(source: &str) -> crate::ast::ReturnItem {
        let Statement::Return(statement) = parse(source).expect("parse succeeds");
        assert_eq!(statement.items.len(), 1);
        statement.items.into_iter().next().unwrap()
    }

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
    fn parse_return_float() {
        let item = only_item("RETURN 1.5");
        assert_eq!(
            item.expr,
            ValueExpr::Literal(Literal::Float(1.5, SourceSpan::new(7, 3)))
        );
    }

    #[test]
    fn parse_return_string() {
        let item = only_item("RETURN 'hello'");
        assert_eq!(
            item.expr,
            ValueExpr::Literal(Literal::String(
                intern("hello").expect("intern succeeds"),
                SourceSpan::new(7, 7)
            ))
        );
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
    fn parse_return_alias() {
        let item = only_item("RETURN 1 AS one");
        assert_eq!(item.alias, Some(intern("one").expect("intern succeeds")));
        assert_eq!(item.span, SourceSpan::new(7, 8));
    }

    #[test]
    fn parse_return_multiple_items() {
        let Statement::Return(statement) = parse("RETURN 1, 2.5, 'x'").expect("parse succeeds");
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
    fn malformed_inputs_return_syntax_error() {
        for source in ["RETURN", "RETURN 1 AS", "RTRN 1", ""] {
            assert!(matches!(
                parse(source),
                Err(ParserError::SyntaxError { .. })
            ));
        }
    }

    #[test]
    fn unsupported_grammar_surface_returns_not_implemented() {
        let err = parse("MATCH (n) RETURN n").expect_err("unsupported grammar should error");
        assert!(matches!(err, ParserError::NotImplemented { .. }));
        assert_eq!(err.gqlstatus(), GqlStatus::FEATURE_NOT_SUPPORTED);
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
    fn parse_return_signed_float() {
        assert_eq!(
            only_item("RETURN -1.5").expr,
            ValueExpr::Literal(Literal::Float(-1.5, SourceSpan::new(7, 4)))
        );
        assert_eq!(
            only_item("RETURN +0.25").expr,
            ValueExpr::Literal(Literal::Float(0.25, SourceSpan::new(7, 5)))
        );
    }

    #[test]
    fn signed_integer_overflow_reports_syntax_error() {
        // `-9223372036854775808` is i64::MIN; pest produces unary(-) over the
        // unsigned magnitude `9223372036854775808`, which doesn't fit in i64.
        // BRIEF-17 will fold the sign into int_lit decoding; for now this is a
        // bounded, intentional limitation surfaced as a syntax error.
        let err =
            parse("RETURN -9223372036854775808").expect_err("magnitude overflow should error");
        assert!(matches!(err, ParserError::SyntaxError { .. }));
    }

    #[test]
    fn parse_return_alias_quoted() {
        let item = only_item("RETURN 1 AS \"my name\"");
        assert_eq!(
            item.alias,
            Some(intern("my name").expect("intern succeeds"))
        );
    }

    #[test]
    fn parse_return_alias_quoted_doubled_quote() {
        let item = only_item("RETURN 1 AS \"a\"\"b\"");
        assert_eq!(item.alias, Some(intern("a\"b").expect("intern succeeds")));
    }

    #[test]
    fn parse_return_alias_backtick() {
        let item = only_item("RETURN 1 AS `my name`");
        assert_eq!(
            item.alias,
            Some(intern("my name").expect("intern succeeds"))
        );
    }

    #[test]
    fn malformed_underscores_in_integer_rejected() {
        for source in ["RETURN 1__2", "RETURN 1_"] {
            let err = parse(source).expect_err("malformed underscores should error");
            assert!(
                matches!(err, ParserError::SyntaxError { .. }),
                "expected SyntaxError for {source:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn non_decimal_literal_reports_not_implemented() {
        // Hex/oct/bin/uint/temporal/list literals parse at the grammar level
        // but their builders land in BRIEF-17. Surface them as
        // NotImplemented (0A000), not SyntaxError, so callers can distinguish
        // capability gaps from typos.
        let err = parse("RETURN 0x10").expect_err("hex literal should report not implemented");
        assert!(matches!(err, ParserError::NotImplemented { .. }));
        assert_eq!(err.gqlstatus(), GqlStatus::FEATURE_NOT_SUPPORTED);
    }
}
