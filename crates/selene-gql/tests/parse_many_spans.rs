//! Regression tests for multi-statement parser span rebasing.

use selene_gql::{ParserError, parse_many};

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
