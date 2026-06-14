//! Parser result-clause `*` conformance regressions.

use selene_gql::{ParserError, parse};

#[test]
fn return_star_rejects_group_by() {
    assert_star_rejection_contains("RETURN * GROUP BY ()", "RETURN * cannot specify GROUP BY");
}

#[test]
fn select_star_rejects_group_by() {
    assert_star_rejection_contains(
        "SELECT * FROM MATCH (n) GROUP BY n",
        "SELECT * cannot specify GROUP BY",
    );
}

#[test]
fn select_star_rejects_missing_from_body() {
    for source in ["SELECT *", "SELECT * WHERE true"] {
        assert_star_rejection_contains(source, "SELECT * requires a FROM clause");
    }
}

fn assert_star_rejection_contains(source: &str, expected: &str) {
    let error = parse(source).expect_err(source);
    let ParserError::SyntaxError { message, .. } = error else {
        panic!("{source}: expected syntax error, got {error:?}");
    };
    assert!(
        message.contains(expected),
        "{source}: expected {expected:?} in {message:?}"
    );
}
