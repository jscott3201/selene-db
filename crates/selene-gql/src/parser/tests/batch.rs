use super::*;

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
fn parse_many_rebases_gg21_key_label_set_span() {
    // The explicit GG21 `<...type key label set>` span feeds the planner's
    // IL003 42013/42015 cardinality diagnostics; in a batch it must be
    // rebased to the original source offset, not the segment-local one.
    let source = "CREATE NODE TYPE :Ok (); CREATE NODE TYPE :A & :B => ()";
    let statements = parse_many(source).expect("parse_many succeeds");
    assert_eq!(statements.len(), 2);
    let crate::ast::Statement::Ddl(crate::ast::DdlStatement::CreateNodeType {
        key_label_set, ..
    }) = &statements[1]
    else {
        panic!("expected a CREATE NODE TYPE with an explicit key label set");
    };
    let key_label_set = key_label_set
        .as_ref()
        .expect("explicit `=>` key label set present");
    let second_stmt_offset = u32::try_from(source.find("CREATE NODE TYPE :A").unwrap()).unwrap();
    assert!(
        key_label_set.span.byte_offset >= second_stmt_offset,
        "key-label-set span {} must be rebased into the second statement (>= {})",
        key_label_set.span.byte_offset,
        second_stmt_offset
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
