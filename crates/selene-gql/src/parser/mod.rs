//! Pest-backed GQL parser entry points.
//!
//! The parser admits one GQL program, enforces the string-interner admission
//! budget, builds the public AST with source spans preserved, and runs the
//! Flagger before callers see unsupported syntax. It does not resolve names,
//! infer types, or choose execution behavior; those invariants start at the
//! analyzer. Deferred grammar surfaces return `ParserError::NotImplemented`
//! with v1.0 support guidance. See ISO GQL Clause 14 and Spec 07.

mod budget;
mod builders;
mod guard;
mod many;

use std::sync::Arc;

use pest::{Parser, error::InputLocation};

use crate::{
    ast::{SourceSpan, Statement},
    diagnostic::DiagnosticReport,
    error::ParserError,
    flagger,
};

use self::pest_impl::{GqlParser, Rule};
use budget::{InternerBudget, MAX_NEW_ADMISSIONS_PER_PARSE};

mod pest_impl {
    #![allow(missing_docs)]

    #[derive(pest_derive::Parser)]
    #[grammar = "parser/grammar.pest"]
    pub(crate) struct GqlParser;
}

pub(crate) const MAX_NESTING_DEPTH: u32 = guard::MAX_NESTING_DEPTH;

/// Parse one GQL program.
///
/// # Errors
///
/// Returns [`ParserError::SyntaxError`] for parse failures and
/// [`ParserError::NotImplemented`] for grammar surfaces whose AST builders
/// are not yet supported in v1.0.
#[tracing::instrument(name = "selene.gql.parse", skip(source), fields(source_len = source.len()))]
pub fn parse(source: &str) -> Result<Statement, ParserError> {
    guard::validate(source)?;
    let mut pairs =
        GqlParser::parse(Rule::gql_program, source).map_err(|error| pest_error(source, error))?;
    let program_pair = pairs.next().ok_or_else(ParserError::empty_program)?;
    let mut budget = InternerBudget::new(MAX_NEW_ADMISSIONS_PER_PARSE);
    let statement = builders::build_statement(program_pair, &mut budget)?;
    flagger::flag(&statement)?;
    Ok(statement)
}

/// Parse one GQL program and wrap failures with source text for miette rendering.
///
/// # Errors
///
/// Returns [`DiagnosticReport`] when parsing, AST construction, admission
/// budgeting, or Flagger validation fails.
pub fn parse_with_source(
    source: Arc<str>,
    label: impl Into<String>,
) -> Result<Statement, DiagnosticReport> {
    parse(&source).map_err(|error| DiagnosticReport::new(error, source, label))
}

pub use many::parse_many;

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
    use super::*;
    use crate::ast::{
        BinaryOp, EdgeDirection, LabelExpr, Literal, PipelineStatement, SetOp, ValueExpr,
    };
    use crate::error::GqlStatus;

    fn query(source: &str) -> crate::ast::QueryPipeline {
        let Statement::Query(query) = parse(source).expect("parse succeeds") else {
            panic!("expected query statement");
        };
        query
    }

    fn return_clause(source: &str) -> crate::ast::ReturnClause {
        let query = query(source);
        assert_eq!(query.statements.len(), 1);
        let PipelineStatement::Return(clause) = query.statements.into_iter().next().unwrap() else {
            panic!("expected return clause");
        };
        clause
    }

    fn only_item(source: &str) -> crate::ast::ReturnItem {
        let clause = return_clause(source);
        assert_eq!(clause.items.len(), 1);
        clause.items.into_iter().next().unwrap()
    }

    fn optional_name(value: Option<selene_core::IStr>) -> Option<&'static str> {
        value.map(selene_core::IStr::as_str)
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
        let ValueExpr::Literal(Literal::String(value, span)) = item.expr else {
            panic!("expected string literal");
        };
        assert_eq!(value.as_str(), "hello");
        assert_eq!(span, SourceSpan::new(7, 7));
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
        assert_eq!(optional_name(item.alias), Some("one"));
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
    fn malformed_inputs_return_syntax_error() {
        for source in ["RETURN", "RETURN 1 AS", "RTRN 1", ""] {
            assert!(matches!(
                parse(source),
                Err(ParserError::SyntaxError { .. })
            ));
        }
    }

    #[test]
    fn parse_inline_call_subquery() {
        let query = query("CALL { RETURN 1 }");
        assert_eq!(query.statements.len(), 1);
        let PipelineStatement::CallSubquery(call) = &query.statements[0] else {
            panic!("expected inline CALL subquery");
        };
        assert!(call.variable_scope.is_none());
        assert!(call.yield_items.is_empty());
        assert!(!call.in_transactions);
        assert_eq!(call.body.statements.len(), 1);
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
        // Signed numeric literals are parsed as unary expressions; reject a
        // bare magnitude that overflows i64 as a syntax error.
        let err =
            parse("RETURN -9223372036854775808").expect_err("magnitude overflow should error");
        assert!(matches!(err, ParserError::SyntaxError { .. }));
    }

    #[test]
    fn parse_return_alias_quoted() {
        let item = only_item("RETURN 1 AS \"my name\"");
        assert_eq!(optional_name(item.alias), Some("my name"));
    }

    #[test]
    fn parse_return_alias_quoted_doubled_quote() {
        let item = only_item("RETURN 1 AS \"a\"\"b\"");
        assert_eq!(optional_name(item.alias), Some("a\"b"));
    }

    #[test]
    fn parse_return_alias_backtick() {
        let item = only_item("RETURN 1 AS `my name`");
        assert_eq!(optional_name(item.alias), Some("my name"));
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
        // Hex/oct/bin/uint/temporal literals parse at the grammar level but
        // their builders land later. Surface them as NotImplemented (42N01),
        // not SyntaxError, so callers can distinguish capability gaps from
        // typos.
        let err = parse("RETURN 0x10").expect_err("hex literal should report not implemented");
        assert!(matches!(err, ParserError::NotImplemented { .. }));
        assert_eq!(err.gqlstatus(), GqlStatus::FEATURE_NOT_SUPPORTED);
    }

    #[test]
    fn parse_match_return_pipeline() {
        let query = query("MATCH (n:Person {age: 42}) WHERE n.active RETURN n.name AS name");
        assert_eq!(query.statements.len(), 2);
        let PipelineStatement::Match(match_clause) = &query.statements[0] else {
            panic!("expected MATCH");
        };
        assert_eq!(match_clause.selector, None);
        assert!(match_clause.where_clause.is_some());
        let node = match &match_clause.patterns[0].elements[0] {
            crate::ast::PatternElement::Node(node) => node,
            _ => panic!("expected node pattern"),
        };
        assert_eq!(optional_name(node.binding), Some("n"));
        let Some(LabelExpr::Single(label)) = node.label_expr.as_ref() else {
            panic!("expected Person label");
        };
        assert_eq!(label.as_str(), "Person");
        assert_eq!(node.properties.len(), 1);

        let PipelineStatement::Return(return_clause) = &query.statements[1] else {
            panic!("expected RETURN");
        };
        assert_eq!(optional_name(return_clause.items[0].alias), Some("name"));
        assert!(matches!(
            return_clause.items[0].expr,
            ValueExpr::PropertyAccess { .. }
        ));
    }

    #[test]
    fn parse_edge_quantifier_and_undirected_direction() {
        let query = query("MATCH (a)-[:KNOWS*1..3]-(b) RETURN b");
        let PipelineStatement::Match(match_clause) = &query.statements[0] else {
            panic!("expected MATCH");
        };
        let edge = match &match_clause.patterns[0].elements[1] {
            crate::ast::PatternElement::Edge(edge) => edge,
            _ => panic!("expected edge pattern"),
        };
        assert_eq!(edge.direction, EdgeDirection::Undirected);
        assert_eq!(
            edge.quantifier,
            Some(crate::ast::Quantifier::GraphPattern {
                min: 1,
                max: Some(3)
            })
        );
    }

    #[test]
    fn parse_label_conjunction_with_colon_separator() {
        let query = query("MATCH (n:Person:Engineer) RETURN n");
        let PipelineStatement::Match(match_clause) = &query.statements[0] else {
            panic!("expected MATCH");
        };
        let node = match &match_clause.patterns[0].elements[0] {
            crate::ast::PatternElement::Node(node) => node,
            _ => panic!("expected node pattern"),
        };
        assert!(matches!(
            node.label_expr,
            Some(LabelExpr::Conjunction(ref parts)) if parts.len() == 2
        ));
    }

    #[test]
    fn parse_binary_expression_precedence() {
        let item = only_item("RETURN 1 + 2 * 3");
        let ValueExpr::BinaryOp {
            op: BinaryOp::Add,
            rhs,
            ..
        } = item.expr
        else {
            panic!("expected addition");
        };
        assert!(matches!(
            *rhs,
            ValueExpr::BinaryOp {
                op: BinaryOp::Mul,
                ..
            }
        ));
    }

    #[test]
    fn parse_function_aggregate_star_and_distinct() {
        let count_star = only_item("RETURN count(*)").expr;
        assert!(matches!(
            count_star,
            ValueExpr::FunctionCall {
                star: true,
                distinct: false,
                ref args,
                ..
            } if args.is_empty()
        ));

        let count_distinct = only_item("RETURN count(DISTINCT n)").expr;
        assert!(matches!(
            count_distinct,
            ValueExpr::FunctionCall {
                star: false,
                distinct: true,
                ref args,
                ..
            } if args.len() == 1
        ));

        let percentile = only_item("RETURN percentile_cont(n, 0.5)").expr;
        assert!(matches!(
            percentile,
            ValueExpr::FunctionCall {
                ref name,
                star: false,
                distinct: false,
                ref args,
                ..
            } if name.len() == 1 && name.first().as_str() == "percentile_cont" && args.len() == 2
        ));
    }

    #[test]
    fn parse_list_record_and_case_expressions() {
        assert!(matches!(
            only_item("RETURN [1, 2][0]").expr,
            ValueExpr::ListAccess { .. }
        ));
        assert!(matches!(
            only_item("RETURN {name: 'Alice'}").expr,
            ValueExpr::RecordLiteral { ref fields, .. } if fields.len() == 1
        ));
        assert!(matches!(
            only_item("RETURN CASE WHEN true THEN 1 ELSE 0 END").expr,
            ValueExpr::Case { ref branches, else_branch: Some(_), .. } if branches.len() == 1
        ));
    }

    #[test]
    fn parse_predicate_expression_family() {
        assert!(matches!(
            only_item("RETURN n IS NOT NULL").expr,
            ValueExpr::IsCheck { negated: true, .. }
        ));
        assert!(matches!(
            only_item("RETURN n.name STARTS WITH 'A'").expr,
            ValueExpr::BinaryOp {
                op: BinaryOp::StartsWith,
                ..
            }
        ));
        assert!(matches!(
            only_item("RETURN PROPERTY_EXISTS(n, 'name')").expr,
            ValueExpr::PropertyExists { .. }
        ));
    }

    #[test]
    fn non_iso_sql_drift_predicates_are_syntax_errors() {
        // `LIKE` and `BETWEEN` are SQL drift with native ISO replacements
        // (STARTS WITH / ENDS WITH / CONTAINS and `x >= lo AND x <= hi`); the
        // grammar must reject them outright rather than accept-then-flag.
        for source in [
            "RETURN n.name LIKE 'a%'",
            "RETURN n.name NOT LIKE 'a%'",
            "RETURN n.age BETWEEN 1 AND 3",
            "RETURN n.age NOT BETWEEN 1 AND 3",
        ] {
            let err = parse(source).expect_err(source);
            assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
        }
    }

    #[test]
    fn non_iso_modulo_and_temporal_and_sql_comment_are_syntax_errors() {
        // `%` infix modulo (use ISO `MOD(x, y)`), `.prop AT TIME ...` temporal
        // access, and the SQL `--` line comment are all non-ISO and removed.
        for source in [
            "RETURN 5 % 2",
            "RETURN n.created AT TIME 'UTC'",
            "RETURN 1 -- trailing comment",
        ] {
            let err = parse(source).expect_err(source);
            assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
        }

        // The ISO replacements and remaining comment forms still parse.
        assert!(matches!(
            only_item("RETURN MOD(5, 2) AS m").expr,
            ValueExpr::FunctionCall { .. }
        ));
        parse("RETURN 1 // trailing comment").expect("// line comment still parses");
        parse("RETURN 1 /* block comment */ AS x").expect("/* */ block comment still parses");
    }

    #[test]
    fn parse_select_desugars_to_return_pipeline() {
        let query = query("SELECT DISTINCT 1 AS one ORDER BY one DESC LIMIT 10");
        assert_eq!(query.statements.len(), 3);
        let PipelineStatement::Return(return_clause) = &query.statements[0] else {
            panic!("expected RETURN");
        };
        assert!(return_clause.distinct);
        assert_eq!(optional_name(return_clause.items[0].alias), Some("one"));
        assert!(matches!(query.statements[1], PipelineStatement::Sorting(_)));
        assert!(matches!(query.statements[2], PipelineStatement::Limit(_)));
    }

    #[test]
    fn parse_composite_and_chained_queries() {
        let Statement::Composite { rest, .. } =
            parse("RETURN 1 UNION ALL RETURN 2").expect("parse succeeds")
        else {
            panic!("expected composite");
        };
        assert_eq!(rest[0].0, SetOp::UnionAll);

        let Statement::Chained { blocks, .. } =
            parse("MATCH (n) RETURN n NEXT MATCH (m) RETURN m").expect("parse succeeds")
        else {
            panic!("expected chained query");
        };
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn intersect_and_except_modifiers_route_to_set_ops() {
        for (source, expected) in [
            ("RETURN 1 INTERSECT RETURN 2", SetOp::Intersect),
            ("RETURN 1 INTERSECT ALL RETURN 2", SetOp::IntersectAll),
            ("RETURN 1 EXCEPT RETURN 2", SetOp::Except),
            ("RETURN 1 EXCEPT ALL RETURN 2", SetOp::ExceptAll),
        ] {
            let Statement::Composite { rest, .. } = parse(source).expect(source) else {
                panic!("expected composite for {source:?}");
            };
            assert_eq!(rest[0].0, expected, "set op for {source:?}");
        }
    }

    #[test]
    fn select_pipeline_emits_pre_return_then_return_then_post_return() {
        // SELECT desugaring must lay statements out in semantic order:
        // pre-projection (MATCH, WHERE/FILTER) before RETURN, post-projection
        // (ORDER BY, OFFSET, LIMIT) after. The previous shape pushed every
        // non-projection clause into a single `deferred` bucket appended
        // after RETURN, which silently rewrote `WHERE` into a post-RETURN
        // filter — wrong semantics whenever projection introduces aliases
        // or aggregation. The grammar's match_stmt currently absorbs an
        // inline WHERE on `FROM MATCH (...) WHERE ...`, so the statement
        // shape here exercises ordering of the post-RETURN slot (sorting
        // and limit) which routes through the same fix.
        let query = query("SELECT n.name FROM MATCH (n) WHERE n.age > 18 ORDER BY n.name LIMIT 10");
        assert!(
            matches!(query.statements[0], PipelineStatement::Match(_)),
            "[0] expected Match, got {:?}",
            query.statements[0]
        );
        assert!(
            matches!(query.statements[1], PipelineStatement::Return(_)),
            "[1] expected Return, got {:?}",
            query.statements[1]
        );
        assert!(
            matches!(query.statements[2], PipelineStatement::Sorting(_)),
            "[2] expected Sorting, got {:?}",
            query.statements[2]
        );
        assert!(
            matches!(query.statements[3], PipelineStatement::Limit(_)),
            "[3] expected Limit, got {:?}",
            query.statements[3]
        );
    }

    #[test]
    fn quantifier_range_with_max_below_min_rejected() {
        for source in [
            "MATCH (a)-[*5..2]-(b) RETURN a",
            "MATCH (a)-[*{5,2}]-(b) RETURN a",
        ] {
            let err = parse(source).expect_err("max < min should error");
            assert!(
                matches!(err, ParserError::SyntaxError { .. }),
                "expected syntax error for {source:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn questioned_quantifier_is_preserved_distinctly() {
        let question_source = "MATCH (a)-[r?]->(b) RETURN r";
        let bounded_source = "MATCH (a)-[r{0,1}]->(b) RETURN r";
        let question_stmt = parse(question_source).expect("parse succeeds");
        let bounded_stmt = parse(bounded_source).expect("parse succeeds");
        assert_eq!(
            crate::ast::format_read_statement(&question_stmt).expect("format succeeds"),
            "MATCH (a)-[r?]->(b)\nRETURN r"
        );
        assert_eq!(
            crate::ast::format_read_statement(&bounded_stmt).expect("format succeeds"),
            "MATCH (a)-[r{0,1}]->(b)\nRETURN r"
        );

        let Statement::Query(question) = question_stmt else {
            panic!("expected query statement");
        };
        let Statement::Query(bounded) = bounded_stmt else {
            panic!("expected query statement");
        };
        let PipelineStatement::Match(question_match) = &question.statements[0] else {
            panic!("expected MATCH");
        };
        let PipelineStatement::Match(bounded_match) = &bounded.statements[0] else {
            panic!("expected MATCH");
        };
        let question_edge = match &question_match.patterns[0].elements[1] {
            crate::ast::PatternElement::Edge(edge) => edge,
            _ => panic!("expected edge pattern"),
        };
        let bounded_edge = match &bounded_match.patterns[0].elements[1] {
            crate::ast::PatternElement::Edge(edge) => edge,
            _ => panic!("expected edge pattern"),
        };
        assert_eq!(
            question_edge.quantifier,
            Some(crate::ast::Quantifier::Questioned)
        );
        assert_eq!(
            bounded_edge.quantifier,
            Some(crate::ast::Quantifier::GraphPattern {
                min: 0,
                max: Some(1)
            })
        );
    }

    #[test]
    fn conflicting_quantifiers_in_edge_pattern_rejected() {
        // edge_interior accepts a quantifier and the outer edge accepts one
        // too. Specifying both must error rather than letting the second
        // silently overwrite the first.
        let err = parse("MATCH (a)-[r*1..2*3..4]->(b) RETURN a")
            .expect_err("conflicting quantifiers should error");
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "expected syntax error, got {err:?}"
        );
    }

    #[test]
    fn is_labeled_with_quoted_keyword_does_not_misroute() {
        // Quoted identifiers that contain IS-suffix keywords (IN, NOT,
        // LIKE, BETWEEN, NORMALIZED, ...) must be classified by grammar
        // rules, not by substring scans of the source text. Otherwise
        // `IS LABELED :"IN"` would be misrouted to the IN predicate and
        // fail with "missing list", and `IS LABELED :"NOT"` would
        // silently flip negation.
        let labeled_in = only_item("RETURN n IS LABELED :\"IN\"").expr;
        let ValueExpr::IsCheck { kind, negated, .. } = labeled_in else {
            panic!("expected IS LABELED to parse as IsCheck");
        };
        assert!(!negated, "no NOT token, but negation flagged");
        assert!(matches!(
            kind,
            crate::ast::IsCheckKind::Labeled(LabelExpr::Single(_))
        ));

        let labeled_not = only_item("RETURN n IS LABELED :\"NOT\"").expr;
        let ValueExpr::IsCheck { negated, .. } = labeled_not else {
            panic!("expected IS LABELED to parse as IsCheck");
        };
        assert!(!negated, "quoted NOT in label name must not flip negation");
    }

    #[test]
    fn is_not_labeled_uses_token_negation() {
        // The NOT keyword in IS NOT LABELED really does negate the predicate.
        let item = only_item("RETURN n IS NOT LABELED :Person").expr;
        let ValueExpr::IsCheck { negated, .. } = item else {
            panic!("expected IS NOT LABELED to parse as IsCheck");
        };
        assert!(negated, "IS NOT LABELED must produce negated=true");
    }

    #[test]
    fn qualified_function_names_preserve_segment_boundaries() {
        // `foo."bar.baz"` and `foo.bar.baz` would collide if the AST stored
        // the qualified name as a single dotted string. The Vec<IStr> path
        // keeps them distinguishable so namespaced procedure calls resolve
        // to the right thing.
        let bare = only_item("RETURN foo.bar.baz()").expr;
        let ValueExpr::FunctionCall {
            name: name_three, ..
        } = bare
        else {
            panic!("expected FunctionCall");
        };
        assert_eq!(name_three.len(), 3);

        let quoted = only_item("RETURN foo.\"bar.baz\"()").expr;
        let ValueExpr::FunctionCall { name: name_two, .. } = quoted else {
            panic!("expected FunctionCall");
        };
        assert_eq!(name_two.len(), 2);

        // Single-segment bare name is still one segment.
        let single = only_item("RETURN count(*)").expr;
        let ValueExpr::FunctionCall { name: name_one, .. } = single else {
            panic!("expected FunctionCall");
        };
        assert_eq!(name_one.len(), 1);
    }

    #[test]
    fn parse_many_single_statement() {
        let statements = parse_many("MATCH (n) RETURN n;").expect("parse_many succeeds");
        assert_eq!(statements.len(), 1);
        assert!(matches!(statements[0], Statement::Query(_)));
    }

    #[test]
    fn parse_many_three_statements_rebases_spans() {
        let source = "INSERT (:A) FINISH;  INSERT (:B) FINISH; INSERT (:C) FINISH";
        let statements = parse_many(source).expect("parse_many succeeds");
        assert_eq!(statements.len(), 3);
        assert!(matches!(statements[0], Statement::Mutate(_)));
        assert_eq!(
            statements[1].span().byte_offset,
            u32::try_from(source.find("INSERT (:B)").unwrap()).unwrap()
        );
    }

    #[test]
    fn parse_many_skips_empty_statements() {
        let statements = parse_many(";;INSERT (:A) FINISH;;").expect("parse_many succeeds");
        assert_eq!(statements.len(), 1);
        assert!(matches!(statements[0], Statement::Mutate(_)));
    }

    #[test]
    fn parse_many_handles_semicolon_in_single_quote() {
        let statements =
            parse_many("RETURN 'a;b' AS value; RETURN 1 AS n").expect("parse_many succeeds");
        assert_eq!(statements.len(), 2);
    }

    #[test]
    fn parse_many_handles_semicolon_in_block_comment() {
        let statements =
            parse_many("/* a;b */ RETURN 1 AS n; RETURN 2 AS n").expect("parse_many succeeds");
        assert_eq!(statements.len(), 2);
        assert_eq!(
            statements[0].span().byte_offset,
            u32::try_from("/* a;b */ ".len()).unwrap()
        );
    }

    #[test]
    fn parse_many_propagates_error_with_rebased_span() {
        let source = "RETURN 1 AS ok; MATCH (n RETURN n";
        let error = parse_many(source).expect_err("second statement should fail");
        let ParserError::SyntaxError { span, .. } = error else {
            panic!("expected syntax error");
        };
        assert!(
            span.byte_offset >= u32::try_from(source.find("MATCH").unwrap()).unwrap(),
            "error span {span:?} should point into the second statement"
        );
    }
}
