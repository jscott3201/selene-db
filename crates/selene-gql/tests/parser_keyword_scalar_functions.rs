//! ISO scalar-function keyword parsing and AST formatting coverage.

use selene_gql::{
    ast::{format_read_statement, structurally_eq},
    parse,
};

#[test]
fn iso_function_heads_are_reserved_but_delimited_identifiers_still_parse() {
    for source in [
        "RETURN left",
        "RETURN RIGHT",
        "RETURN abs",
        "RETURN CARDINALITY",
        "RETURN labels",
        "RETURN ELEMENTS",
        "RETURN NORMALIZE",
        "RETURN DATETIME",
        "RETURN DURATION_BETWEEN",
        "RETURN ZONED_DATETIME",
    ] {
        assert!(
            parse(source).is_err(),
            "{source} must reject a bare keyword head"
        );
    }

    for source in [
        "RETURN \"left\" AS value",
        "RETURN \"ABS\" AS value",
        "RETURN \"CARDINALITY\" AS value",
        "RETURN \"NORMALIZE\" AS value",
    ] {
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    }
}

#[test]
fn reserved_iso_function_heads_remain_callable() {
    for source in [
        "RETURN left('abcdef', 2) AS value",
        "RETURN RIGHT(X'CAFE', 1) AS value",
        "RETURN char_length('abc') AS value",
        "RETURN BYTE_LENGTH(X'CA') AS value",
        "RETURN ABS(-3) AS value",
        "RETURN MOD(7, 4) AS value",
        "RETURN SIN(0) AS value",
        "RETURN FLOOR(1.5D) AS value",
        "RETURN BTRIM(' x ') AS value",
        "RETURN ELEMENTS(p) AS value",
        "RETURN LABELS(n) AS value",
        "RETURN NORMALIZE('e') AS value",
        "RETURN PROPERTY_EXISTS(n, 'name') AS value",
        "RETURN ALL_DIFFERENT(1, 2) AS value",
        "RETURN SAME(n, n) AS value",
        "RETURN ELEMENT_ID(null) AS value",
        "RETURN COALESCE(null, 1) AS value",
        "RETURN NULLIF(1, 1) AS value",
        "RETURN CURRENT_DATE AS value",
        "RETURN CURRENT_TIME AS value",
        "RETURN CURRENT_TIMESTAMP AS value",
        "RETURN DATE('2026-01-01') AS value",
        "RETURN DATETIME('2026-01-01T00:00:00') AS value",
        "RETURN LOCAL_DATETIME('2026-01-01T00:00:00') AS value",
        "RETURN TIME('12:34:56') AS value",
        "RETURN LOCAL_TIME('12:34:56') AS value",
        "RETURN ZONED_TIME('12:34:56Z') AS value",
        "RETURN ZONED_DATETIME('2026-01-01T00:00:00Z') AS value",
        "RETURN DURATION(null) AS value",
        "RETURN DURATION_BETWEEN(DATE('2026-01-01'), DATE('2026-01-02')) AS value",
    ] {
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    }
}

#[test]
fn dedicated_temporal_value_functions_reject_generic_call_shapes() {
    for source in [
        "RETURN CURRENT_DATE() AS value",
        "RETURN CURRENT_TIME() AS value",
        "RETURN CURRENT_TIMESTAMP() AS value",
        "RETURN DURATION_BETWEEN(null) AS value",
    ] {
        assert!(
            parse(source).is_err(),
            "{source} must reject non-ISO generic-call syntax"
        );
    }
}

#[test]
fn bare_duration_remains_value_head_but_not_type_name() {
    for source in [
        "RETURN DURATION 'PT1H' AS value",
        "RETURN DURATION('PT1H') AS value",
        "RETURN DURATION({days: 1}) AS value",
        "RETURN CAST('PT1H' AS DURATION (DAY TO SECOND)) AS value",
        "RETURN DURATION 'P2M' IS TYPED DURATION (YEAR TO MONTH) AS ok",
    ] {
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    }

    for source in [
        "RETURN CAST('PT1H' AS DURATION) AS value",
        "RETURN DURATION 'PT1H' IS TYPED DURATION AS ok",
        "CREATE NODE TYPE :Event (span :: DURATION)",
    ] {
        assert!(
            parse(source).is_err(),
            "{source} must reject non-ISO bare DURATION type syntax"
        );
    }
}

#[test]
fn keyword_function_calls_format_bare_and_round_trip() {
    for (source, quoted_head) in [
        ("RETURN left('abcdef', 2) AS value", "\"left\"("),
        ("RETURN abs(-3) AS value", "\"abs\"("),
        ("RETURN char_length('abc') AS value", "\"char_length\"("),
        ("RETURN trim(' x ') AS value", "\"trim\"("),
        ("RETURN labels(n) AS value", "\"labels\"("),
        ("RETURN element_id(null) AS value", "\"element_id\"("),
        ("RETURN coalesce(null, 1) AS value", "\"coalesce\"("),
        ("RETURN current_date AS value", "\"current_date\""),
        ("RETURN date(null) AS value", "\"date\"("),
        ("RETURN duration(null) AS value", "\"duration\"("),
    ] {
        let parsed = parse(source).unwrap_or_else(|error| panic!("{source} parses: {error:?}"));
        let formatted = format_read_statement(&parsed).expect("read-side AST formats");
        assert!(
            !formatted.contains(quoted_head),
            "keyword function call should stay bare: {formatted}"
        );
        let reparsed =
            parse(&formatted).unwrap_or_else(|error| panic!("{formatted} reparses: {error:?}"));
        assert!(
            structurally_eq(&parsed, &reparsed),
            "{source} should round-trip through {formatted}"
        );
    }
}

#[test]
fn reserved_keyword_aliases_still_format_as_identifiers() {
    for keyword in [
        "LEFT",
        "ABS",
        "CARDINALITY",
        "NORMALIZE",
        "ELEMENT_ID",
        "COALESCE",
        "CURRENT_DATE",
        "DATE",
        "LOCAL_TIME",
        "DURATION",
        "IMPLIES",
        "PATHS",
        "SUBSTRING",
        "CURRENT_GRAPH",
        "HOME_SCHEMA",
        "INTEGER8",
        "FLOAT32",
    ] {
        let source = format!("RETURN 1 AS \"{keyword}\"");
        let parsed = parse(&source).unwrap_or_else(|error| panic!("{source} parses: {error:?}"));
        let formatted = format_read_statement(&parsed).expect("read-side AST formats");
        assert_eq!(formatted, source);
    }
}

#[test]
fn implemented_reserved_function_and_temporal_heads_reject_bare_aliases() {
    for keyword in [
        "ELEMENT_ID",
        "COALESCE",
        "NULLIF",
        "CURRENT_DATE",
        "CURRENT_TIME",
        "CURRENT_TIMESTAMP",
        "DATE",
        "DATETIME",
        "LOCAL_DATETIME",
        "LOCAL_TIME",
        "ZONED_DATETIME",
        "ZONED_TIME",
        "DURATION",
        "DURATION_BETWEEN",
        "TIMESTAMP",
        "LOCAL_TIMESTAMP",
        "ZONED",
        "WITHOUT",
    ] {
        let source = format!("RETURN 1 AS {keyword}");
        assert!(
            parse(&source).is_err(),
            "{source} must reject a bare reserved alias"
        );
    }
}

#[test]
fn iso_reserved_type_session_path_and_pre_reserved_words_reject_bare_identifiers() {
    let reserved_words = [
        "ASCENDING",
        "DESCENDING",
        "BIG",
        "CHAR",
        "BINARY",
        "FLOAT32",
        "INTEGER8",
        "INT8",
        "UINT8",
        "UNSIGNED",
        "PARAMETER",
        "PARAMETERS",
        "SESSION",
        "RESET",
        "CLOSE",
        "CHARACTERISTICS",
        "IF",
        "NOTHING",
        "ON",
        "UNIQUE",
        "IMPLIES",
        "PATH",
        "PATHS",
        "LIKE",
        "SUBSTRING",
        "CURRENT_GRAPH",
        "HOME_SCHEMA",
    ];

    for keyword in reserved_words {
        let expression = format!("RETURN {keyword}");
        assert!(
            parse(&expression).is_err(),
            "{expression} must reject a bare reserved expression"
        );
    }

    for keyword in reserved_words.into_iter().chain(["INTEGER"]) {
        let alias = format!("RETURN 1 AS {keyword}");
        assert!(
            parse(&alias).is_err(),
            "{alias} must reject a bare reserved alias"
        );
    }

    for source in [
        "RETURN \"PATH\" AS \"SUBSTRING\"",
        "RETURN \"CURRENT_GRAPH\" AS \"HOME_SCHEMA\"",
    ] {
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    }
}

#[test]
fn prefix_overlapping_keywords_reject_bare_aliases() {
    for keyword in ["ASC", "ASIN", "INTERSECT", "NULLIF", "NULLS", "NORMALIZED"] {
        let source = format!("RETURN 1 AS {keyword}");
        assert!(
            parse(&source).is_err(),
            "{source} must reject prefix-overlapping reserved alias"
        );
    }
}

#[test]
fn literal_keyword_prefixes_remain_identifier_names() {
    for source in [
        "RETURN TRUEVALUE AS value",
        "RETURN FALSEVALUE AS value",
        "RETURN UNKNOWNVALUE AS value",
        "RETURN NULLVALUE AS value",
        "RETURN NFCVALUE AS value",
    ] {
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    }
}
