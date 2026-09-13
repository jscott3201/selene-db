//! Compatibility of the factored query and IN alternatives.

use selene_gql::{ParserError, PipelineStatement, SetOp, SourceSpan, Statement, ValueExpr, parse};

fn first_projection(source: &str) -> ValueExpr {
    let Statement::Query(query) = parse(source).expect("query parses") else {
        panic!("expected plain query");
    };
    let PipelineStatement::Return(mut clause) = query.statements.into_iter().next().unwrap() else {
        panic!("expected RETURN");
    };
    clause.items.remove(0).expr
}

#[test]
fn factored_query_preserves_variants_arms_and_spans() {
    let source = "RETURN 1 UNION ALL RETURN 2 EXCEPT RETURN 3";
    let Statement::Composite { first, rest, span } = parse(source).unwrap() else {
        panic!("expected composite");
    };
    assert_eq!(span, SourceSpan::new(0, source.len() as u32));
    assert_eq!(first.statements.len(), 1);
    assert_eq!(first.span.byte_offset, 0);
    assert_eq!(rest.len(), 2);
    assert_eq!(rest[0].0, SetOp::UnionAll);
    assert_eq!(rest[1].0, SetOp::Except);
    assert_eq!(rest[0].1.span.byte_offset, 19);
    assert_eq!(rest[1].1.span.byte_offset, 35);

    let source = "RETURN 1 NEXT RETURN 2 NEXT RETURN 3";
    let Statement::Chained { blocks, span } = parse(source).unwrap() else {
        panic!("expected chained query");
    };
    assert_eq!(span, SourceSpan::new(0, source.len() as u32));
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].span.byte_offset, 0);
    assert_eq!(blocks[1].span.byte_offset, 14);
    assert_eq!(blocks[2].span.byte_offset, 28);

    for source in [
        "RETURN 1",
        "AT /s RETURN 1",
        "{ RETURN 1 }",
        "USE g RETURN 1",
    ] {
        let Statement::Query(query) = parse(source).unwrap() else {
            panic!("expected plain query: {source}");
        };
        assert_eq!(query.statements.len(), 1);
        assert_eq!(query.span, SourceSpan::new(0, source.len() as u32));
    }
}

#[test]
fn mixed_next_and_set_tails_remain_syntax_errors_in_every_query_host() {
    for operator in ["UNION", "UNION ALL", "INTERSECT", "EXCEPT", "OTHERWISE"] {
        for body in [
            format!("RETURN 1 {operator} RETURN 2 NEXT RETURN 3"),
            format!("RETURN 1 NEXT RETURN 2 {operator} RETURN 3"),
        ] {
            for source in [
                body.clone(),
                format!("AT /s {body}"),
                format!("{{ {body} }}"),
                format!("USE g {{ {body} }}"),
                format!("RETURN VALUE {{ {body} }}"),
                format!("CALL {{ {body} }}"),
                format!("EXPLAIN {body}"),
            ] {
                let error = parse(&source).expect_err("mixed tails are not GQL in this grammar");
                assert!(
                    matches!(error, ParserError::SyntaxError { .. }),
                    "{source}: {error:?}"
                );
                assert_eq!(error.gqlstatus().as_str(), "42001");
            }
        }
    }
}

#[test]
fn at_and_nested_composition_keep_the_specification_diagnostic() {
    for body in ["RETURN 1 UNION RETURN 2", "RETURN 1 NEXT RETURN 2"] {
        for (prefix, suffix) in [
            ("AT /s ", ""),
            ("{ ", " }"),
            ("USE g { ", " }"),
            ("RETURN VALUE { ", " }"),
            ("CALL { ", " }"),
        ] {
            let source = format!("{prefix}{body}{suffix}");
            let error = parse(&source).expect_err("nested composition is not implemented");
            assert_eq!(error.gqlstatus().as_str(), "42N01");
            let ParserError::NotImplemented { span, .. } = error else {
                panic!("expected specification diagnostic");
            };
            // Pest may include trailing trivia, but not the host or closing brace.
            let highlighted = &source[span.byte_offset as usize..span.end() as usize];
            assert_eq!(highlighted.trim_end(), body);
        }
    }
}

#[test]
fn in_rhs_keeps_direct_list_commitment_and_expression_variants() {
    for source in ["RETURN 0 IN [0]", "RETURN 0 NOT IN /* list */ [0]"] {
        assert!(matches!(first_projection(source), ValueExpr::InList { .. }));
    }
    for source in [
        "RETURN 0 IN ([0])",
        "RETURN 0 IN -[0]",
        "RETURN 0 IN $items",
        "RETURN 0 IN ([0] || [1])",
    ] {
        assert!(matches!(
            first_projection(source),
            ValueExpr::InListExpression { .. }
        ));
    }
    // The old direct-list branch committed before any postfix or comparison
    // continuation. Factoring must not broaden it to the full comparison RHS.
    for source in [
        "RETURN 0 IN [0] + 1",
        "RETURN 0 IN [0] || [1]",
        "RETURN 0 IN [0] = [1]",
        "RETURN 0 IN [0].x",
    ] {
        assert!(matches!(
            parse(source),
            Err(ParserError::SyntaxError { .. })
        ));
    }
}

#[test]
fn concurrent_parses_keep_independent_quote_decisions() {
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                for _ in 0..8 {
                    for source in [
                        "RETURN \"f\"()",
                        "RETURN `f`(0)",
                        "RETURN \"f\", VALUE {RETURN 0}",
                        "RETURN 1 UNION RETURN 2",
                    ] {
                        parse(source).expect("parser state belongs to this invocation");
                    }
                }
            });
        }
    });
}
