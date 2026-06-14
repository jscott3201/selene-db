use super::*;
use crate::ast::{
    BinaryOp, BindingTableType, CharacterStringLiteralKind, EdgeDirection, GqlType,
    IntegerLiteralKind, IsCheckKind, LabelExpr, Literal, PipelineStatement,
    RowExpansionPositionKind, RowExpansionSyntax, SetOp, ValueExpr,
};
use crate::error::GqlStatus;

mod batch;
mod numeric;
mod radix;

fn query(source: &str) -> crate::ast::QueryPipeline {
    let Statement::Query(query) = parse(source).expect("parse succeeds") else {
        panic!("expected query statement");
    };
    query
}

fn parse_unflagged(source: &str) -> Statement {
    guard::validate(source).expect("source passes parser guard");
    let mut pairs = GqlParser::parse(Rule::gql_program, source).expect("pest parse succeeds");
    let program_pair = pairs.next().expect("program pair exists");
    builders::build_statement(program_pair).expect("AST build succeeds")
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

fn only_unflagged_item(source: &str) -> crate::ast::ReturnItem {
    let Statement::Query(query) = parse_unflagged(source) else {
        panic!("expected query statement");
    };
    assert_eq!(query.statements.len(), 1);
    let PipelineStatement::Return(clause) = query.statements.into_iter().next().unwrap() else {
        panic!("expected return clause");
    };
    assert_eq!(clause.items.len(), 1);
    clause.items.into_iter().next().unwrap()
}

fn optional_name(value: Option<selene_core::DbString>) -> Option<String> {
    value.map(|name| name.as_str().to_owned())
}

fn assert_function_call(source: &str, expected_name: &str) {
    assert_function_call_with_args(source, expected_name, 0);
}

fn assert_function_call_with_args(source: &str, expected_name: &str, expected_args: usize) {
    let expr = only_item(source).expr;
    let ValueExpr::FunctionCall {
        ref name,
        ref args,
        star,
        distinct,
        ..
    } = expr
    else {
        panic!("expected function call expression for {source}");
    };
    assert_eq!(name.len(), 1, "{source}");
    assert_eq!(name.first().as_str(), expected_name, "{source}");
    assert_eq!(args.len(), expected_args, "{source}");
    assert!(!star, "{source}");
    assert!(!distinct, "{source}");
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
fn parse_for_list_statement_as_row_expansion() {
    let query = query("FOR x IN [1, 2] RETURN x");
    assert_eq!(query.statements.len(), 2);
    let PipelineStatement::Unwind(statement) = &query.statements[0] else {
        panic!("expected row expansion");
    };
    assert_eq!(statement.syntax, RowExpansionSyntax::For);
    assert_eq!(statement.alias.as_str(), "x");
    let ValueExpr::ListLiteral { items, .. } = &statement.source else {
        panic!("expected list source");
    };
    assert_eq!(items.len(), 2);
}

#[test]
fn parse_for_position_tail() {
    for (source, expected) in [
        (
            "FOR x IN [1, 2] WITH ORDINALITY ord RETURN x, ord",
            RowExpansionPositionKind::Ordinality,
        ),
        (
            "FOR x IN [1, 2] WITH OFFSET off RETURN x, off",
            RowExpansionPositionKind::Offset,
        ),
    ] {
        let query = query(source);
        let PipelineStatement::Unwind(statement) = &query.statements[0] else {
            panic!("expected row expansion");
        };
        let position = statement.position.as_ref().expect("position tail parses");
        assert_eq!(position.kind, expected);
    }
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
fn signed_integer_overflow_reports_syntax_error() {
    // `-9223372036854775808` is i64::MIN; pest produces unary(-) over the
    // unsigned magnitude `9223372036854775808`, which doesn't fit in i64.
    // Signed numeric literals are parsed as unary expressions; reject a
    // bare magnitude that overflows i64 as a syntax error.
    let err = parse("RETURN -9223372036854775808").expect_err("magnitude overflow should error");
    assert!(matches!(err, ParserError::SyntaxError { .. }));
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
fn parse_temporal_literal() {
    let item = only_item("RETURN DATE '2020-01-01'");
    let ValueExpr::Literal(Literal::Date(value, span, kind)) = item.expr else {
        panic!("expected DATE literal");
    };
    assert_eq!(value.to_string(), "2020-01-01");
    assert_eq!(span, SourceSpan::new(7, 17));
    assert_eq!(kind, CharacterStringLiteralKind::Escaped);
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
    assert_eq!(optional_name(node.binding.clone()).as_deref(), Some("n"));
    let Some(LabelExpr::Single(label)) = node.label_expr.as_ref() else {
        panic!("expected Person label");
    };
    assert_eq!(label.as_str(), "Person");
    assert_eq!(node.properties.len(), 1);

    let PipelineStatement::Return(return_clause) = &query.statements[1] else {
        panic!("expected RETURN");
    };
    assert_eq!(
        optional_name(return_clause.items[0].alias.clone()).as_deref(),
        Some("name")
    );
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
    } = &item.expr
    else {
        panic!("expected addition");
    };
    assert!(matches!(
        **rhs,
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

    assert!(matches!(
        only_item("RETURN percentile_cont(DISTINCT n, 0.5)").expr,
        ValueExpr::FunctionCall {
            ref name,
            star: false,
            distinct: true,
            ref args,
            ..
        } if name.len() == 1 && name.first().as_str() == "percentile_cont" && args.len() == 2
    ));
}

#[test]
fn parse_current_datetime_keyword_functions() {
    assert_function_call("RETURN CURRENT_DATE", "current_date");
    assert_function_call("RETURN CURRENT_TIME", "current_time");
    assert_function_call("RETURN CURRENT_TIMESTAMP", "current_timestamp");
    assert_function_call("RETURN LOCAL_TIMESTAMP", "local_datetime");
    assert_function_call("RETURN LOCAL_TIME", "local_time");
    assert_function_call("RETURN LOCAL_TIME()", "local_time");

    assert_function_call("RETURN local_datetime()", "local_datetime");
    assert_function_call("RETURN local_time()", "local_time");
    assert_function_call_with_args("RETURN LOCAL_TIME('12:34:56')", "local_time", 1);

    for source in [
        "RETURN CURRENT_DATE()",
        "RETURN CURRENT_TIME()",
        "RETURN CURRENT_TIMESTAMP()",
    ] {
        assert!(parse(source).is_err(), "{source} must reject parentheses");
    }
}

#[test]
fn parse_rejects_non_count_aggregate_star_shapes() {
    for source in [
        "RETURN sum(*)",
        "RETURN count(DISTINCT *)",
        "RETURN avg(*)",
        "RETURN collect_list(*)",
    ] {
        let err = parse(source).expect_err("invalid aggregate star shape should reject");
        assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
    }
}

#[test]
fn parse_rejects_zero_argument_aggregates() {
    for source in [
        "RETURN count()",
        "RETURN sum()",
        "RETURN collect_list(DISTINCT)",
    ] {
        let err = parse(source).expect_err("aggregate without value expression should reject");
        assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
    }
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
    assert_eq!(
        optional_name(return_clause.items[0].alias.clone()).as_deref(),
        Some("one")
    );
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
fn binding_table_reference_type_preserves_field_types_before_feature_gate() {
    let item = only_unflagged_item(
        "RETURN NULL IS TYPED BINDING TABLE { id :: INT, payload :: RECORD { ok :: BOOL } } AS ok",
    );
    let ValueExpr::IsCheck {
        kind: IsCheckKind::Typed(GqlType::TableRef(BindingTableType::Closed(fields))),
        ..
    } = &item.expr
    else {
        panic!("expected typed predicate over closed binding table reference type");
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].0.as_str(), "id");
    assert_eq!(fields[0].1, GqlType::Integer);
    assert_eq!(fields[1].0.as_str(), "payload");
    assert!(matches!(fields[1].1, GqlType::Record(_)));
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
    let ValueExpr::IsCheck { kind, negated, .. } = &labeled_in else {
        panic!("expected IS LABELED to parse as IsCheck");
    };
    assert!(!negated, "no NOT token, but negation flagged");
    assert!(matches!(
        kind,
        crate::ast::IsCheckKind::Labeled(LabelExpr::Single(_))
    ));

    let labeled_not = only_item("RETURN n IS LABELED :\"NOT\"").expr;
    let ValueExpr::IsCheck { negated, .. } = &labeled_not else {
        panic!("expected IS LABELED to parse as IsCheck");
    };
    assert!(!negated, "quoted NOT in label name must not flip negation");
}

#[test]
fn is_not_labeled_uses_token_negation() {
    // The NOT keyword in IS NOT LABELED really does negate the predicate.
    let item = only_item("RETURN n IS NOT LABELED :Person").expr;
    let ValueExpr::IsCheck { negated, .. } = &item else {
        panic!("expected IS NOT LABELED to parse as IsCheck");
    };
    assert!(negated, "IS NOT LABELED must produce negated=true");
}

#[test]
fn qualified_function_names_preserve_segment_boundaries() {
    // `foo."bar.baz"` and `foo.bar.baz` would collide if the AST stored
    // the qualified name as a single dotted string. The Vec<DbString> path
    // keeps them distinguishable so namespaced procedure calls resolve
    // to the right thing.
    let bare = only_item("RETURN foo.bar.baz()").expr;
    let ValueExpr::FunctionCall {
        name: name_three, ..
    } = &bare
    else {
        panic!("expected FunctionCall");
    };
    assert_eq!(name_three.len(), 3);

    let quoted = only_item("RETURN foo.\"bar.baz\"()").expr;
    let ValueExpr::FunctionCall { name: name_two, .. } = &quoted else {
        panic!("expected FunctionCall");
    };
    assert_eq!(name_two.len(), 2);

    // Single-segment bare name is still one segment.
    let single = only_item("RETURN count(*)").expr;
    let ValueExpr::FunctionCall { name: name_one, .. } = &single else {
        panic!("expected FunctionCall");
    };
    assert_eq!(name_one.len(), 1);
}

// PARSER-DOS complexity-guard unit tests live in the dedicated integration
// suite `tests/parser_dos_artifacts.rs` (the `dos` regression module),
// alongside the embedded fuzz artifacts, so this near-cap module file stays
// comfortably under the 700-LOC gate. The guard maps to GQLSTATUS 5GQL1
// PROGRAM_LIMIT_EXCEEDED per ISO/IEC 39075:2024 section 23.1; see
// `crate::error::GqlStatus::PROGRAM_LIMIT_EXCEEDED` for the Table 8 grounding.
