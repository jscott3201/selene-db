//! Compare lookahead boundaries with the authoritative grammar on bounded inputs.

use pest::Parser;

use super::{FALLBACK_CAPACITY, WORK_CAPACITY, end, skip_trivia};
use crate::parser::{GqlParser, Rule};

fn assert_grammar_boundary(source: &str) {
    let expected = GqlParser::parse(Rule::type_name, source)
        .ok()
        .and_then(|mut pairs| pairs.next())
        .map(|pair| skip_trivia(source.as_bytes(), pair.as_span().end()));
    let actual = end(source, 0)
        .expect("bounded fixture fits the lookahead arrays")
        .map(|cursor| skip_trivia(source.as_bytes(), cursor));
    assert_eq!(actual, expected, "{source}");
}

#[test]
fn scalar_and_structural_tails_match_the_type_grammar() {
    for ty in [
        "BOOLEAN",
        "BOOL",
        "SIGNED SMALL INTEGER",
        "SIGNED BIG INTEGER",
        "SIGNED INTEGER",
        "SIGNED INTEGER(10)",
        "UNSIGNED INTEGER(10)",
        "UNSIGNED SMALL INTEGER",
        "UNSIGNED BIG INTEGER",
        "INT(8)",
        "UINT(8)",
        "BIG INTEGER",
        "SMALL INTEGER",
        "BIGINT",
        "SMALLINT",
        "USMALLINT",
        "UBIGINT",
        "DOUBLE PRECISION",
        "DOUBLE",
        "FLOAT(12,2)",
        "FLOAT(1_2)",
        "REAL",
        "DECIMAL(10,2)",
        "DEC(10)",
        "STRING(0xA,0b10000)",
        "CHAR(0o12)",
        "VARCHAR(1_0)",
        "BYTES(4,8)",
        "BINARY(4)",
        "VARBINARY(4)",
        "BYTEA",
        "UUID",
        "JSON",
        "VECTOR",
        "TIMESTAMP",
        "TIMESTAMP WITH TIME ZONE",
        "TIMESTAMP WITHOUT TIME ZONE",
        "TIME WITH TIME ZONE",
        "TIME WITHOUT TIME ZONE",
        "LOCAL DATETIME",
        "ZONED DATETIME",
        "LOCAL TIME",
        "ZONED TIME",
        "DATE",
        "DURATION(YEAR TO MONTH)",
        "DURATION(DAY TO SECOND)",
        "GRAPH",
        "NODE",
        "VERTEX",
        "EDGE",
        "RELATIONSHIP",
        "ANY GRAPH",
        "ANY PROPERTY GRAPH",
        "ANY NODE",
        "ANY VERTEX",
        "ANY EDGE",
        "ANY RELATIONSHIP",
        "PROPERTY GRAPH",
        "PATH",
        "LIST",
        "ARRAY[0xA]",
        "LIST<INT>[4]",
        "INT NOT NULL ARRAY[2] LIST NOT NULL",
        "INT | STRING NOT NULL",
        "RECORD",
        "RECORD {}",
        "ANY RECORD",
        "{}",
        "RECORD {x :: LIST<INT>, y TYPED {z STRING}}",
        "{`é``😊` INT, \"字\" JSON}",
        "TABLE {}",
        "BINDING TABLE {x INT}",
        "ANY",
        "ANY VALUE",
        "PROPERTY VALUE",
        "ANY PROPERTY VALUE",
        "ANY<INT | LIST<ANY VALUE<STRING | BOOL>>>",
        "NULL",
        "NOTHING",
        "LIST[1 /* digit */ 2]",
        "LIST[0x /* digit */ A _ B]",
        "STRING(1__0_)",
    ] {
        for source in [
            format!("{ty} ;"),
            format!("RECORD {{field {ty}}} {{RETURN 0}}"),
        ] {
            assert_grammar_boundary(&source);
        }
    }
    for prefix in [
        "INT",
        "INTEGER",
        "UINT",
        "SIGNED INTEGER",
        "UNSIGNED INTEGER",
        "FLOAT",
    ] {
        for width in [8, 16, 32, 64, 128, 256] {
            assert_grammar_boundary(&format!("RECORD {{x {prefix}{width}}} {{RETURN 0}}"));
        }
    }
}

#[test]
fn malformed_optional_fields_and_primary_unions_match_the_type_grammar() {
    for tail in [
        "JSON(0)",
        "BOOL(1)",
        "INT(1,2)",
        "INT(0x10)",
        "INT(1__0)",
        "INT(1_)",
        "CHAR(1,2)",
        "BYTES(1,2,3)",
        "STRING(0XA)",
        "STRING(1 2)",
        "STRING(0xA_)",
        "DOUBLE INT",
        "SIGNED INT",
        "UNSIGNED INT8",
        "SMALL INT",
        "FLOAT8",
        "TIME",
        "TIME ZONE",
        "TIMESTAMP WITH ZONE",
        "LOCAL",
        "ZONED DATE",
        "DURATION()",
        "DURATION(YEAR TO SECOND)",
        "ANY VALUE GRAPH",
        "ANY PROPERTY RECORD",
        "LIST[x]",
        "LIST[1,2]",
        "LIST[1+2]",
        "LIST[1_]",
        "LIST[0xA_]",
        "LIST[]",
        "INT | ANY<INT>",
        "ANY<ANY<INT>>",
        "ANY<INT> ARRAY",
        "ANY<INT> NOT NULL",
        "ANY<INT> | STRING",
        "INT |",
        "LIST<>",
        "LIST<ANY<>>",
        "NULL,",
        "{x INT,}",
        "{x : : INT}",
        "{x :: TYPED INT}",
        "{\"\" INT}",
        "{😊 INT}",
    ] {
        for source in [
            tail.to_owned(),
            format!("RECORD {{RETURN {tail}}} {{RETURN 0}}"),
        ] {
            assert_grammar_boundary(&source);
        }
    }
}

#[test]
fn type_lookahead_preserves_utf8_and_eof_boundaries() {
    for source in [
        "RECORD {`é``😊` LIST<INT>[0x10]} /* tail */",
        "ANY<INT | RECORD {\"字\" DURATION(YEAR TO MONTH)}>",
        "RECORD {x :: STRING(1_0), y BINDING TABLE {z INT}}",
    ] {
        for (cursor, _) in source.char_indices() {
            assert_grammar_boundary(&source[..cursor]);
        }
        assert_grammar_boundary(source);
    }
}

#[test]
fn supported_type_depth_and_wide_unions_fit_bounded_storage() {
    for (prefix, suffix, count) in [
        ("LIST<", ">", 64),
        // Both a prefixed union and its LIST component add an AST type level.
        ("ANY<INT | LIST<", ">>", 32),
        ("INT | RECORD {x ", "}", 64),
    ] {
        let source = format!("{}GRAPH{}", prefix.repeat(count), suffix.repeat(count));
        assert_eq!(end(&source, 0).unwrap(), Some(source.len()));
    }
    let source = format!("{}GRAPH", "INT | ".repeat(1024));
    assert_eq!(end(&source, 0).unwrap(), Some(source.len()));
}

#[test]
fn each_capacity_failure_is_distinct_from_an_incomplete_type() {
    let work = format!("{}GRAPH", "LIST<".repeat(128));
    assert_eq!(end(&work, 0).unwrap_err().limit, WORK_CAPACITY as u32);
    let fallbacks = format!("{}GRAPH", "ANY<INT | LIST<".repeat(128));
    assert_eq!(
        end(&fallbacks, 0).unwrap_err().limit,
        FALLBACK_CAPACITY as u32
    );
    assert!(end("BINDING TABLE {x", 0).unwrap().is_none());
    // Capacity exhaustion inside an optional RECORD attempt cannot fall back
    // to the bare RECORD prefix and expose an unguarded query to pest.
    assert!(end(&format!("RECORD {{x {work}"), 0).is_err());
}
