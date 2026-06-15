use super::*;

#[test]
fn byte_string_left_and_right_follow_iso_substring_rules() {
    assert_eq!(
        first_value("RETURN left(X'CAFE00', 2) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN right(X'CAFE00', 2) AS payload"),
        bytes(&[0xfe, 0x00])
    );
    assert_eq!(
        first_value("RETURN left(X'CAFE', 99) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN right(X'CAFE', 0) AS payload"),
        bytes(&[])
    );
    assert_eq!(
        first_value("RETURN left(X'CAFE00', CAST('2' AS INT128)) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN right(X'CAFE00', CAST('2' AS UINT128)) AS payload"),
        bytes(&[0xfe, 0x00])
    );
    assert_eq!(
        first_value("RETURN left(X'CAFE00', 2M) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN left(X'CAFE', null) AS payload"),
        Value::Null
    );
    assert_eq!(first_value("RETURN left(null, 1) AS payload"), Value::Null);
    assert_eq!(first_status("RETURN left(X'CAFE', -1) AS payload"), "22011");
    assert_eq!(
        first_status("RETURN left(X'CAFE', 1.5M) AS payload"),
        "22G03"
    );
}

#[test]
fn byte_string_trim_follows_iso_trim_rules() {
    assert_eq!(
        first_value("RETURN TRIM(X'20CA20') AS payload"),
        bytes(&[0xca])
    );
    assert_eq!(
        first_value("RETURN TRIM(BOTH X'00' FROM X'0000CAFE00') AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN TRIM(LEADING X'00' FROM X'0000CAFE00') AS payload"),
        bytes(&[0xca, 0xfe, 0x00])
    );
    assert_eq!(
        first_value("RETURN TRIM(TRAILING X'00' FROM X'0000CAFE00') AS payload"),
        bytes(&[0x00, 0x00, 0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN TRIM(BOTH FROM X'20CA20') AS payload"),
        bytes(&[0xca])
    );
    assert_eq!(
        first_value("RETURN TRIM(BOTH X'00' FROM null) AS payload"),
        Value::Null
    );
    assert_eq!(
        first_value("RETURN TRIM(BOTH null FROM X'0000') AS payload"),
        Value::Null
    );
    assert_eq!(
        first_status("RETURN TRIM(BOTH X'' FROM X'CA') AS payload"),
        "22027"
    );
    assert_eq!(
        first_status("RETURN TRIM(BOTH X'CAFE' FROM X'CA') AS payload"),
        "22027"
    );
    assert_eq!(
        first_status("RETURN TRIM(BOTH 'x' FROM X'78') AS payload"),
        "22G03"
    );
    assert_eq!(
        first_status("RETURN TRIM(BOTH X'78' FROM 'xx') AS payload"),
        "22G03"
    );
}

#[test]
fn byte_string_trim_infers_bytes_and_records_gf07() {
    assert_eq!(
        projection_type(
            &analyze_one("RETURN TRIM(BOTH X'00' FROM X'00CA00') AS payload"),
            "payload"
        ),
        AnalyzedType::Resolved(GqlType::Bytes)
    );
    assert_feature_recorded(
        "RETURN TRIM(BOTH X'00' FROM X'00CA00') AS payload",
        FeatureId::GF07,
    );
    assert_feature_recorded("RETURN TRIM(X'20CA20') AS payload", FeatureId::GF07);
}

#[test]
fn bounded_byte_string_trim_and_concat_infer_plain_bytes() {
    assert_eq!(
        projection_type(
            &analyze_one("RETURN X'CA' || CAST(X'FE' AS BINARY(1)) AS payload"),
            "payload"
        ),
        AnalyzedType::Resolved(GqlType::Bytes)
    );
    assert_eq!(
        first_value("RETURN X'CA' || CAST(X'FE' AS BINARY(1)) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        projection_type(
            &analyze_one("RETURN TRIM(BOTH CAST(X'00' AS BINARY(1)) FROM X'00CA00') AS payload"),
            "payload"
        ),
        AnalyzedType::Resolved(GqlType::Bytes)
    );
}

#[test]
fn byte_string_concatenation_uses_byte_string_result_type() {
    assert_eq!(
        first_value("RETURN X'CA' || X'FE00' AS payload"),
        bytes(&[0xca, 0xfe, 0x00])
    );
    assert_eq!(
        first_value("RETURN X'' || X'CA' AS payload"),
        bytes(&[0xca])
    );
    assert_eq!(first_status("RETURN X'CA' || 'FE' AS payload"), "22G03");
}

#[test]
fn byte_string_concatenation_truncates_only_zero_byte_overflow() {
    let caps = ImplDefinedCaps::default().with_max_byte_string_length(3);
    assert_eq!(
        first_value_with_caps("RETURN X'CAFE' || X'0000' AS payload", caps),
        bytes(&[0xca, 0xfe, 0x00])
    );
}

#[test]
fn byte_string_concatenation_reports_right_truncation() {
    let caps = ImplDefinedCaps::default().with_max_byte_string_length(1);
    assert_eq!(
        first_status_with_caps("RETURN X'CA' || X'FE' AS payload", caps),
        "22001"
    );
}
