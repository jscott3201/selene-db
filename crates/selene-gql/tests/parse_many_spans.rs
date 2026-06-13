//! Regression tests for multi-statement parser span rebasing.

use selene_gql::{ParserError, Statement, parse_many};

/// Byte offset of the Nth statement's `span()` in the parsed result.
fn span_offset(statements: &[Statement], index: usize) -> u32 {
    statements[index].span().byte_offset
}

#[test]
fn double_quote_ident_semicolon_does_not_split() {
    // PARSE-18: a `;` inside a double-quoted (delimited) identifier must not be
    // a statement boundary. The DoubleQuote scanner state was previously
    // uncovered for the not-split invariant.
    let source = "RETURN 1 AS \"a;b\"; RETURN 2 AS n";
    let second_start = source.rfind("RETURN ").unwrap() as u32;
    let statements = parse_many(source).expect("delimited-ident semicolon is not a boundary");
    assert_eq!(statements.len(), 2, "the in-ident `;` must not split");
    assert_eq!(span_offset(&statements, 1), second_start);
}

#[test]
fn double_quoted_string_escaped_quote_semicolon_does_not_split() {
    // ISO character string literals can be double-quoted. A semicolon after an
    // escaped double quote is still inside the string and must not become a
    // statement boundary.
    let source = r#"RETURN "a\";b" AS s; RETURN 2 AS n"#;
    let second_start = source.rfind("RETURN ").unwrap() as u32;
    let statements = parse_many(source).expect("double-quoted string semicolon is not a boundary");
    assert_eq!(statements.len(), 2, "the in-string `;` must not split");
    assert_eq!(span_offset(&statements, 1), second_start);
}

#[test]
fn backtick_ident_semicolon_does_not_split() {
    // PARSE-18: the Backtick scanner state — `;` inside a backtick-quoted
    // identifier is not a boundary.
    let source = "RETURN 1 AS `a;b`; RETURN 2 AS n";
    let second_start = source.rfind("RETURN ").unwrap() as u32;
    let statements = parse_many(source).expect("backtick-ident semicolon is not a boundary");
    assert_eq!(statements.len(), 2);
    assert_eq!(span_offset(&statements, 1), second_start);
}

#[test]
fn backtick_doubled_backtick_semicolon_does_not_split() {
    let source = "RETURN 1 AS `a``;b`; RETURN 2 AS n";
    let second_start = source.rfind("RETURN ").unwrap() as u32;
    let statements = parse_many(source).expect("backtick-ident semicolon is not a boundary");
    assert_eq!(statements.len(), 2);
    assert_eq!(span_offset(&statements, 1), second_start);
}

#[test]
fn double_slash_line_comment_semicolon_does_not_split() {
    // PARSE-18: the LineComment scanner state entered via `//`. A `;` inside the
    // comment is not a boundary; the real `;` after the newline is.
    let source = "RETURN 1 AS n // tail ; not a boundary\n; RETURN 2 AS n";
    let second_start = source.rfind("RETURN ").unwrap() as u32;
    let statements = parse_many(source).expect("`//` comment semicolon is not a boundary");
    assert_eq!(
        statements.len(),
        2,
        "the in-`//`-comment `;` must not split"
    );
    assert_eq!(span_offset(&statements, 1), second_start);
}

#[test]
fn dash_dash_line_comment_semicolon_does_not_split() {
    // PARSE-18: the LineComment scanner state entered via `--`. The boundary
    // scanner treats `--` as a line comment, so a `;` inside it is NOT a
    // boundary. (`--` itself is non-ISO and pest rejects it, so the second
    // segment fails as ONE unit — proving the inner `;` did not carve off a
    // separately-parsed third segment: the error span lands inside the second
    // statement, not at the post-comment text.)
    let source = "RETURN 1 AS ok; RETURN 2 AS n -- tail ; still comment\nAND more";
    let second_start = source.rfind("RETURN ").unwrap() as u32;
    let inner_semicolon = source.find("; still comment").unwrap() as u32;
    let error = parse_many(source).expect_err("`--` is non-ISO; the second segment fails to parse");
    let ParserError::SyntaxError { span, .. } = error else {
        panic!("expected SyntaxError, got {error:?}");
    };
    assert!(
        span.byte_offset >= second_start,
        "error span {span:?} should be inside the second statement (>= {second_start})"
    );
    assert!(
        span.byte_offset < inner_semicolon,
        "error span {span:?} should be BEFORE the in-comment `;` ({inner_semicolon}); a split there \
         would have produced a different (separately-parsed) segment"
    );
}

#[test]
fn escaped_single_quote_semicolon_does_not_split() {
    // PARSE-18: backslash-escaped single quote inside a single-quoted string.
    // The scanner skips the escaped quote (advancing 2), so the string stays
    // open and the `;` inside it is not a boundary.
    let source = "RETURN 'a\\';b' AS s; RETURN 2 AS n";
    let second_start = source.rfind("RETURN ").unwrap() as u32;
    let statements = parse_many(source).expect("escaped-quote semicolon is not a boundary");
    assert_eq!(statements.len(), 2, "the in-string `;` must not split");
    assert_eq!(span_offset(&statements, 1), second_start);
}

#[test]
fn doubled_single_quote_semicolon_does_not_split() {
    // PARSE-18: the doubled-`''` escape form inside a single-quoted string. The
    // scanner consumes `''` as a literal quote, keeping the string open across
    // the following `;`.
    let source = "RETURN 'a'';b' AS s; RETURN 2 AS n";
    let second_start = source.rfind("RETURN ").unwrap() as u32;
    let statements = parse_many(source).expect("doubled-quote semicolon is not a boundary");
    assert_eq!(statements.len(), 2);
    assert_eq!(span_offset(&statements, 1), second_start);
}

#[test]
fn parse_many_error_span_points_into_original_second_statement() {
    let source = "RETURN 1 AS ok; MATCH (n RETURN n";
    let second_start = source.find("MATCH").expect("fixture has second statement") as u32;
    let error = parse_many(source).expect_err("second statement should fail");
    let ParserError::SyntaxError { span, .. } = error else {
        panic!("expected syntax error");
    };

    assert!(
        span.byte_offset >= second_start,
        "error span {span:?} should be rebased into the second statement"
    );
    assert!(
        span.byte_offset < source.len() as u32,
        "error span {span:?} should stay within the original source"
    );
}
