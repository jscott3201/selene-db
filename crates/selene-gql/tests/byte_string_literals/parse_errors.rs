use super::*;

#[test]
fn bounded_byte_string_type_bounds_reject_invalid_lengths() {
    for source in [
        "RETURN X'CA' IS TYPED BYTES(0) AS ok",
        "RETURN X'CA' IS TYPED BYTES(4, 2) AS ok",
        "RETURN X'CA' IS TYPED BINARY(0) AS ok",
        "RETURN X'CA' IS TYPED VARBINARY(0) AS ok",
    ] {
        parse(source).expect_err("invalid byte-string bounds reject");
    }
}

#[test]
fn byte_string_literal_formatter_canonicalizes_uppercase_hex() {
    let parsed = parse("RETURN x'00 ff a5' AS payload").expect("source parses");
    let formatted = format_read_statement(&parsed).expect("formats");
    assert_eq!(formatted, "RETURN X'00FFA5' AS payload");
    let reparsed = parse(&formatted).expect("formatted source reparses");
    assert!(structurally_eq(&parsed, &reparsed));
}

#[test]
fn byte_string_literal_rejects_odd_hex_digit_count() {
    let err = parse("RETURN X'0' AS payload").expect_err("odd hex count is rejected");
    let ParserError::SyntaxError { message, .. } = err else {
        panic!("expected syntax error");
    };
    assert!(message.contains("odd number of hexadecimal digits"));
}

#[test]
fn byte_string_literal_rejects_plain_space_between_chunks() {
    parse("RETURN X'CA' 'FE' AS payload").expect_err("chunk separator requires a newline");
}

#[test]
fn byte_string_literal_rejects_non_hex_digit() {
    parse("RETURN X'0G' AS payload").expect_err("non-hex digit is rejected");
}

#[test]
fn non_identity_bytes_casts_are_invalid_type_combinations() {
    for source in [
        "RETURN CAST('00' AS BYTES) AS payload",
        "RETURN CAST(1 AS BYTES) AS payload",
        "RETURN CAST(true AS BYTES) AS payload",
        "RETURN CAST(X'CAFE' AS STRING) AS payload",
        "RETURN CAST(X'CAFE' AS INTEGER) AS payload",
        "RETURN CAST(X'CAFE' AS BOOLEAN) AS payload",
        "RETURN CAST([X'CAFE'] AS LIST<STRING>) AS payload",
    ] {
        assert_eq!(first_status(source), "22G03", "{source}");
    }
}
