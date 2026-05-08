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
/// Returns [`ParserError::SyntaxError`] for parse failures and for grammar
/// surfaces whose AST builders intentionally land after BRIEF-16.
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
    fn unsupported_grammar_surface_returns_visible_error() {
        assert!(matches!(
            parse("MATCH (n) RETURN n"),
            Err(ParserError::SyntaxError { .. })
        ));
    }
}
