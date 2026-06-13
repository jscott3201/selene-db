//! Negative string-literal escape diagnostics.

mod exec_common;

use exec_common::{column_values, execute_read};
use selene_core::Value;
use selene_gql::{ParserError, parse};

fn string_value(source: &str) -> String {
    let table = execute_read(source);
    let mut values = column_values(&table, "value");
    assert_eq!(values.len(), 1);
    let value = values.pop().expect("one value");
    let Value::String(value) = value else {
        panic!("expected string value, got {value:?}");
    };
    value.to_string()
}

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
    parse(r#"RETURN "\"" AS value"#).expect("backslash-escaped double quote parses");
    parse(r#"RETURN """" AS value"#).expect("doubled double quote parses");
}

#[test]
fn mid_string_quote_escape_parses() {
    parse("RETURN 'a\\'b' AS value").expect("mid-string \\' parses");
    parse(r#"RETURN "a\"b" AS value"#).expect("mid-string \\\" parses");
}

#[test]
fn double_quoted_string_literals_decode() {
    for (source, expected) in [
        (r#"RETURN "plain" AS value"#, "plain"),
        (r#"RETURN "a""b" AS value"#, "a\"b"),
        (r#"RETURN "a\"b" AS value"#, "a\"b"),
        (r#"RETURN "a\nb" AS value"#, "a\nb"),
        (r#"RETURN "\u00E9\U0001F600" AS value"#, "\u{00e9}\u{1f600}"),
        (r#"RETURN "don't" AS value"#, "don't"),
    ] {
        assert_eq!(string_value(source), expected, "{source}");
    }
}

#[test]
fn unterminated_string_escape_rejected() {
    assert_syntax_contains("RETURN '\\' AS value", "unterminated string escape");
}

#[test]
fn unknown_string_escape_rejected() {
    assert_syntax_contains(r"RETURN '\q' AS value", "unknown string escape");
    assert_syntax_contains(r#"RETURN "\q" AS value"#, "unknown string escape");
}

#[test]
fn unterminated_unicode_escape_rejected() {
    assert_syntax_contains(r"RETURN '\u123' AS value", "unterminated unicode escape");
    assert_syntax_contains(r#"RETURN "\u123" AS value"#, "unterminated unicode escape");
}

#[test]
fn invalid_unicode_hex_digit_rejected() {
    assert_syntax_contains(r"RETURN '\uXYZW' AS value", "invalid unicode escape");
    assert_syntax_contains(r#"RETURN "\uXYZW" AS value"#, "invalid unicode escape");
}

#[test]
fn invalid_unicode_scalar_rejected() {
    assert_syntax_contains(r"RETURN '\uD800' AS value", "invalid unicode scalar");
    assert_syntax_contains(r"RETURN '\UFFFFFFFF' AS value", "invalid unicode scalar");
}
