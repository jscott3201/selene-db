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
    ] {
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
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
    for keyword in ["LEFT", "ABS", "CARDINALITY", "NORMALIZE"] {
        let source = format!("RETURN 1 AS \"{keyword}\"");
        let parsed = parse(&source).unwrap_or_else(|error| panic!("{source} parses: {error:?}"));
        let formatted = format_read_statement(&parsed).expect("read-side AST formats");
        assert_eq!(formatted, source);
    }
}
