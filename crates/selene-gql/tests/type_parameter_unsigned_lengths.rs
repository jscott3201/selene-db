//! ISO unsigned-integer type-parameter coverage.

use selene_gql::{ParserError, ast::format_read_statement, parse};

#[test]
fn character_string_lengths_accept_iso_unsigned_integer_tokens() {
    for (source, expected_format) in [
        (
            "RETURN 'abc' IS TYPED STRING(0x1, 0x4) AS ok",
            "RETURN 'abc' IS TYPED STRING(1, 4) AS ok",
        ),
        (
            "RETURN 'abc' IS TYPED STRING(1, 1_0) AS ok",
            "RETURN 'abc' IS TYPED STRING(1, 10) AS ok",
        ),
        (
            "RETURN 'abc' IS TYPED VARCHAR(0b100) AS ok",
            "RETURN 'abc' IS TYPED STRING(4) AS ok",
        ),
        (
            "RETURN 'ab' IS TYPED CHAR(0o2) AS ok",
            "RETURN 'ab' IS TYPED STRING(2, 2) AS ok",
        ),
    ] {
        assert_canonical_format(source, expected_format);
    }

    for source in [
        "RETURN 'abc' IS TYPED STRING(1_) AS ok",
        "RETURN 'abc' IS TYPED STRING(1__2) AS ok",
        "RETURN 'abc' IS TYPED VARCHAR(0x_) AS ok",
    ] {
        assert_malformed_length_rejected(source);
    }
}

#[test]
fn byte_string_lengths_accept_iso_unsigned_integer_tokens() {
    for (source, expected_format) in [
        (
            "RETURN X'CAFE' IS TYPED BYTES(0x1, 0x4) AS ok",
            "RETURN X'CAFE' IS TYPED BYTES(1, 4) AS ok",
        ),
        (
            "RETURN X'CAFE' IS TYPED BYTES(1, 1_0) AS ok",
            "RETURN X'CAFE' IS TYPED BYTES(1, 10) AS ok",
        ),
        (
            "RETURN X'CAFE' IS TYPED VARBINARY(0b100) AS ok",
            "RETURN X'CAFE' IS TYPED BYTES(4) AS ok",
        ),
        (
            "RETURN X'CAFE' IS TYPED BINARY(0o2) AS ok",
            "RETURN X'CAFE' IS TYPED BYTES(2, 2) AS ok",
        ),
    ] {
        assert_canonical_format(source, expected_format);
    }

    for source in [
        "RETURN X'CAFE' IS TYPED BYTES(1_) AS ok",
        "RETURN X'CAFE' IS TYPED BYTES(1__2) AS ok",
        "RETURN X'CAFE' IS TYPED VARBINARY(0x_) AS ok",
    ] {
        assert_malformed_length_rejected(source);
    }
}

#[test]
fn bounded_string_type_lengths_accept_comment_boundaries() {
    for (source, expected_format) in [
        (
            "RETURN 'abc' IS TYPED STRING /* c */ ( /* c */ 1 /* c */ , /* c */ 4 /* c */ ) AS ok",
            "RETURN 'abc' IS TYPED STRING(1, 4) AS ok",
        ),
        (
            "RETURN 'ab' IS TYPED CHAR /* c */ ( /* c */ 0o2 /* c */ ) AS ok",
            "RETURN 'ab' IS TYPED STRING(2, 2) AS ok",
        ),
        (
            "RETURN 'abc' IS TYPED VARCHAR /* c */ ( /* c */ 0b100 /* c */ ) AS ok",
            "RETURN 'abc' IS TYPED STRING(4) AS ok",
        ),
        (
            "RETURN X'CAFE' IS TYPED BYTES /* c */ ( /* c */ 1 /* c */ , /* c */ 4 /* c */ ) AS ok",
            "RETURN X'CAFE' IS TYPED BYTES(1, 4) AS ok",
        ),
        (
            "RETURN X'CAFE' IS TYPED BINARY /* c */ ( /* c */ 0o2 /* c */ ) AS ok",
            "RETURN X'CAFE' IS TYPED BYTES(2, 2) AS ok",
        ),
        (
            "RETURN X'CAFE' IS TYPED VARBINARY /* c */ ( /* c */ 0b100 /* c */ ) AS ok",
            "RETURN X'CAFE' IS TYPED BYTES(4) AS ok",
        ),
    ] {
        assert_canonical_format(source, expected_format);
    }
}

#[test]
fn bounded_string_type_keywords_require_boundaries_before_type_suffixes() {
    for source in [
        "RETURN 'abc' IS TYPED STRINGARRAY AS ok",
        "RETURN 'abc' IS TYPED CHARARRAY AS ok",
        "RETURN 'abc' IS TYPED VARCHARARRAY AS ok",
        "RETURN 'abc' IS TYPED STRINGNOT NULL AS ok",
        "RETURN 'abc' IS TYPED CHARNOT NULL AS ok",
        "RETURN 'abc' IS TYPED VARCHARNOT NULL AS ok",
        "RETURN X'CAFE' IS TYPED BYTESARRAY AS ok",
        "RETURN X'CAFE' IS TYPED BINARYARRAY AS ok",
        "RETURN X'CAFE' IS TYPED VARBINARYARRAY AS ok",
        "RETURN X'CAFE' IS TYPED BYTESNOT NULL AS ok",
        "RETURN X'CAFE' IS TYPED BINARYNOT NULL AS ok",
        "RETURN X'CAFE' IS TYPED VARBINARYNOT NULL AS ok",
    ] {
        assert_syntax_error(source);
    }
}

fn assert_canonical_format(source: &str, expected_format: &str) {
    let statement =
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    let formatted = format_read_statement(&statement).expect("formats");
    assert_eq!(formatted, expected_format);
}

fn assert_malformed_length_rejected(source: &str) {
    assert!(
        parse(source).is_err(),
        "{source} must reject malformed unsigned integer length syntax"
    );
}

fn assert_syntax_error(source: &str) {
    let err = parse(source).expect_err("source should reject");
    assert!(
        matches!(err, ParserError::SyntaxError { .. }),
        "{source:?} should reject as syntax, got {err:?}"
    );
}
