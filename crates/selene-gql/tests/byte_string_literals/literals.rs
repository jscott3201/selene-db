use super::*;

#[test]
fn byte_string_literal_executes_to_bytes_value() {
    assert_eq!(
        first_value("RETURN X'00 ff A5' AS payload"),
        bytes(&[0x00, 0xff, 0xa5])
    );
    assert_eq!(first_value("RETURN x'0a' AS payload"), bytes(&[0x0a]));
    assert_eq!(first_value("RETURN X'' AS payload"), bytes(&[]));
}

#[test]
fn byte_string_literal_chunks_concatenate_across_newline_separator() {
    assert_eq!(
        first_value("RETURN X'CA'\n'FE' AS payload"),
        bytes(&[0xca, 0xfe])
    );
}

#[test]
fn byte_string_literal_infers_bytes_and_is_typed_bytes() {
    let analyzed = analyze_one("RETURN X'CAFE' AS payload");
    assert_eq!(
        projection_type(&analyzed, "payload"),
        AnalyzedType::Resolved(GqlType::Bytes)
    );
    assert_eq!(
        first_value("RETURN X'CAFE' IS TYPED BYTES AS ok"),
        Value::Bool(true)
    );
}

#[test]
fn bounded_byte_string_forms_parse_flag_and_format() {
    for (source, expected) in [
        ("RETURN X'CAFE' IS TYPED BYTES(2) AS ok", FeatureId::GV37),
        ("RETURN X'CAFE' IS TYPED BYTES(1, 4) AS ok", FeatureId::GV36),
        ("RETURN X'CAFE' IS TYPED BINARY(2) AS ok", FeatureId::GV38),
        (
            "RETURN X'CAFE' IS TYPED VARBINARY(2) AS ok",
            FeatureId::GV37,
        ),
    ] {
        parse(source).unwrap_or_else(|err| panic!("bounded byte-string type parses: {err:?}"));
        assert_feature_recorded(source, FeatureId::GV35);
        assert_feature_recorded(source, expected);
    }

    let fixed = parse("RETURN X'CAFE' IS TYPED BINARY(2) AS ok").expect("source parses");
    let formatted = format_read_statement(&fixed).expect("formats");
    assert_eq!(formatted, "RETURN X'CAFE' IS TYPED BYTES(2, 2) AS ok");
    let reparsed = parse(&formatted).expect("formatted source reparses");
    assert!(structurally_eq(&fixed, &reparsed));

    let variable = parse("RETURN X'CAFE' IS TYPED VARBINARY(4) AS ok").expect("source parses");
    let formatted = format_read_statement(&variable).expect("formats");
    assert_eq!(formatted, "RETURN X'CAFE' IS TYPED BYTES(4) AS ok");
    let reparsed = parse(&formatted).expect("formatted source reparses");
    assert!(structurally_eq(&variable, &reparsed));
}

#[test]
fn bounded_byte_string_type_predicates_check_length_bounds() {
    assert_eq!(
        first_value("RETURN X'CAFE' IS TYPED BYTES(2) AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN X'CAFE00' IS TYPED BYTES(2) AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN X'CA' IS TYPED BYTES(2, 4) AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN X'CAFE00' IS TYPED BYTES(2, 4) AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN X'CAFE' IS TYPED BINARY(2) AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN X'CA' IS TYPED BINARY(2) AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN X'CAFE00' IS TYPED VARBINARY(2) AS ok"),
        Value::Bool(false)
    );
}

#[test]
fn bounded_byte_string_cast_pads_and_truncates_trailing_zeroes() {
    assert_eq!(
        first_value("RETURN CAST(X'CA' AS BYTES(2, 4)) AS payload"),
        bytes(&[0xca, 0x00])
    );
    assert_eq!(
        first_value("RETURN CAST(X'CA' AS BINARY(3)) AS payload"),
        bytes(&[0xca, 0x00, 0x00])
    );
    assert_eq!(
        first_value("RETURN CAST(X'CAFE00' AS VARBINARY(2)) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN CAST(X'CAFE00' AS BYTES(1, 2)) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_status("RETURN CAST(X'CAFE01' AS VARBINARY(2)) AS payload"),
        "22001"
    );
}

#[test]
fn byte_string_literal_identity_cast_returns_bytes() {
    assert_eq!(
        first_value("RETURN CAST(X'CAFE' AS BYTES) AS payload"),
        bytes(&[0xca, 0xfe])
    );
}

#[test]
fn bare_byte_string_type_aliases_normalize_to_bytes() {
    assert_eq!(
        projection_type(
            &analyze_one("RETURN CAST(X'CAFE' AS BINARY) AS payload"),
            "payload"
        ),
        AnalyzedType::Resolved(GqlType::Bytes)
    );
    assert_eq!(
        first_value("RETURN X'CAFE' IS TYPED BINARY AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN CAST(X'CAFE' AS VARBINARY) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN X'CAFE' IS TYPED VARBINARY AS ok"),
        Value::Bool(true)
    );
}

#[test]
fn typed_parameters_enforce_bounded_byte_string_lengths() {
    assert_eq!(
        first_value_with_parameter(
            "RETURN $payload :: BYTES(2, 4) AS payload",
            "payload",
            bytes(&[0xca, 0xfe])
        ),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_status_with_parameter(
            "RETURN $payload :: BYTES(2, 4) AS payload",
            "payload",
            bytes(&[0xca])
        ),
        "22G03"
    );
}
