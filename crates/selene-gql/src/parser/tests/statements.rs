use super::*;

#[test]
fn parse_inline_call_subquery() {
    let query = query("CALL { RETURN 1 }");
    assert_eq!(query.statements.len(), 1);
    let PipelineStatement::CallSubquery(call) = &query.statements[0] else {
        panic!("expected inline CALL subquery");
    };
    assert!(call.variable_scope.is_none());
    assert!(call.yield_items.is_empty());
    assert_eq!(call.body.statements.len(), 1);
}

#[test]
fn parse_for_list_statement_as_row_expansion() {
    let query = query("FOR x IN [1, 2] RETURN x");
    assert_eq!(query.statements.len(), 2);
    let PipelineStatement::For(statement) = &query.statements[0] else {
        panic!("expected row expansion");
    };
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
        let PipelineStatement::For(statement) = &query.statements[0] else {
            panic!("expected row expansion");
        };
        let position = statement.position.as_ref().expect("position tail parses");
        assert_eq!(position.kind, expected);
    }
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
