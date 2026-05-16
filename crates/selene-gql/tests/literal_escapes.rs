//! Negative string-literal escape diagnostics.

use selene_gql::{ParserError, parse};

fn syntax_message(source: &str) -> String {
    match parse(source) {
        Err(ParserError::SyntaxError { message, .. }) => message,
        other => panic!("expected SyntaxError for {source:?}, got {other:?}"),
    }
}

fn assert_syntax_contains(source: &str, expected: &str) {
    let message = syntax_message(source);
    assert!(
        message.contains(expected),
        "expected {message:?} to contain {expected:?}"
    );
}

#[test]
fn valid_quote_escapes_still_parse() {
    parse("RETURN '\\'' AS value").expect("backslash-escaped quote parses");
    parse("RETURN '''' AS value").expect("doubled quote parses");
}

#[test]
fn unterminated_string_escape_rejected() {
    assert_syntax_contains("RETURN '\\' AS value", "unterminated string escape");
}

#[test]
fn unknown_string_escape_rejected() {
    assert_syntax_contains(r"RETURN '\q' AS value", "unknown string escape");
}

#[test]
fn unterminated_unicode_escape_rejected() {
    assert_syntax_contains(r"RETURN '\u123' AS value", "unterminated unicode escape");
}

#[test]
fn invalid_unicode_hex_digit_rejected() {
    assert_syntax_contains(r"RETURN '\uXYZW' AS value", "invalid unicode escape");
}

#[test]
fn invalid_unicode_scalar_rejected() {
    assert_syntax_contains(r"RETURN '\uD800' AS value", "invalid unicode scalar");
    assert_syntax_contains(r"RETURN '\UFFFFFFFF' AS value", "invalid unicode scalar");
}
