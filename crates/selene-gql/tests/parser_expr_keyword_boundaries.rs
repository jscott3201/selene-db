//! Expression keyword-boundary regression coverage.

use selene_gql::{ParserError, parse};

fn assert_syntax_error(source: &str) {
    let error = parse(source).expect_err(source);
    assert!(
        matches!(error, ParserError::SyntaxError { .. }),
        "{source} must reject as syntax, got {error:?}"
    );
}

#[test]
fn case_branch_keywords_require_boundaries() {
    for source in [
        "RETURN CASE WHENx THEN 1 END",
        "RETURN CASE WHEN true THENx END",
        "RETURN CASE WHEN true THEN 1 ELSEy END",
        "RETURN CASE WHEN true THEN 1 ENDx",
        "RETURN CASE 1 WHENx THEN 'hit' END",
        "RETURN CASE 1 WHEN 1 THENx END",
        "RETURN CASE 1 WHEN 1 THEN 'hit' ELSEy END",
        "RETURN CASE 1 WHEN 1 THEN 'hit' ENDx",
    ] {
        assert_syntax_error(source);
    }
}

#[test]
fn string_match_operator_keywords_require_boundaries() {
    for source in [
        "RETURN 'abc' STARTSWITH 'a' AS v",
        "RETURN 'abc' STARTS WITHx AS v",
        "RETURN 'abc' ENDSWITH 'c' AS v",
        "RETURN 'abc' ENDS WITHx AS v",
        "RETURN 'abc' CONTAINSx AS v",
    ] {
        assert_syntax_error(source);
    }
}

#[test]
fn guarded_expression_keywords_still_accept_iso_forms() {
    for source in [
        "RETURN CASE WHEN true THEN 1 ELSE 0 END",
        "RETURN CASE 1 WHEN 1, 2 THEN 'hit' ELSE 'miss' END",
        "RETURN CASE WHEN true THEN CASE WHEN false THEN 1 ELSE 2 END ELSE 3 END",
        "RETURN 'abc' STARTS WITH 'a' AS v",
        "RETURN 'abc' STARTS /* c */ WITH 'a' AS v",
        "RETURN 'abc' ENDS WITH 'c' AS v",
        "RETURN 'abc' CONTAINS 'b' AS v",
    ] {
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    }
}
