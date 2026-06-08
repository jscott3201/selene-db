//! Read-side AST pretty-printer round-trip property tests.

use proptest::test_runner::TestRunner;
use selene_gql::{
    Statement,
    ast::{FormatError, format_read_statement, structurally_eq},
    parse,
};
use selene_testing::corpus::{CorpusKind, Expectation, load_default_corpus};

#[test]
fn representative_read_shapes_round_trip() {
    for source in [
        "MATCH (n:Person {name: 'Ada'}) RETURN n.name AS name",
        "RETURN (1 + 2) * 3 AS n",
        "RETURN count(*) AS c",
        "RETURN CASE WHEN n.age > 10 THEN 'old' ELSE 'new' END AS bucket",
    ] {
        assert_round_trip(source);
    }
}

#[test]
fn typed_list_predicate_preserves_element_type() {
    // Regression for Codex P2 on PR #24: fmt_type was hard-coding every
    // List(_) to "LIST<STRING>" so `IS TYPED LIST<INT8>` round-tripped
    // into `IS TYPED LIST<STRING>` and structurally-equal failed.
    for source in [
        "RETURN n IS TYPED LIST<INT8>",
        "RETURN n IS TYPED LIST<INTEGER>",
        "RETURN n IS TYPED LIST<DATE>",
        "RETURN n IS TYPED LIST<LIST<INT32>>",
    ] {
        assert_round_trip(source);
    }
}

#[test]
fn reserved_word_aliases_are_quoted_in_formatted_output() {
    // Regression for Codex P2 on PR #24: the formatter's KEYWORDS list
    // was much smaller than the grammar's reserved-word set, so
    // identifiers whose uppercase form is a keyword (DISTINCT, WITH,
    // ASC, MIN, BY, ...) could emit bare and reparse as the keyword
    // instead of as an identifier.
    for source in [
        "RETURN 1 AS \"DISTINCT\"",
        "RETURN 1 AS \"WITH\"",
        "RETURN 1 AS \"ASC\"",
        "RETURN 1 AS \"BY\"",
        "RETURN 1 AS \"MIN\"",
        "RETURN 1 AS \"COUNT\"",
        "RETURN 1 AS \"NULL\"",
        "RETURN 1 AS \"AND\"",
    ] {
        assert_round_trip(source);
    }
}

#[test]
fn contextual_keyword_aliases_are_quoted_in_formatted_output() {
    // Contextual grammar tokens still parse as bare identifiers in some slots,
    // so structural round-trip alone cannot prove the formatter made the
    // identifier role explicit. Pin the emitted quotes directly.
    for keyword in [
        "EXPLAIN",
        "INDEXES",
        "PROCEDURES",
        "TRANSACTIONS",
        "VALUE",
        "NORMALIZE",
        "PERCENTILE_CONT",
        "PERCENTILE_DISC",
    ] {
        let source = format!("RETURN 1 AS \"{keyword}\"");
        let parsed = parse(&source).unwrap_or_else(|error| panic!("{source} parses: {error:?}"));
        let formatted = format_read_statement(&parsed).expect("read-side AST formats");
        assert_eq!(formatted, source, "{keyword} alias remains quoted");
        let reparsed = parse(&formatted).expect("formatted source parses");
        assert!(
            structurally_eq(&parsed, &reparsed),
            "{keyword} alias round-trips structurally"
        );
    }
}

#[test]
fn positive_read_corpus_round_trips_under_proptest() {
    let sources = load_default_corpus()
        .expect("corpus loads")
        .into_iter()
        .filter(|case| {
            case.kind == CorpusKind::Positive && case.expectation == Expectation::ParseOk
        })
        .filter_map(|case| {
            let parsed = parse(&case.source).ok()?;
            is_read_side(&parsed).then_some(case.source)
        })
        .collect::<Vec<_>>();
    assert!(!sources.is_empty(), "read-side positive corpus must exist");

    let mut runner = TestRunner::default();
    runner
        .run(&proptest::sample::select(sources), |source| {
            assert_round_trip(&source);
            Ok(())
        })
        .expect("round-trip property holds");
}

#[test]
fn write_side_statements_report_unsupported_with_stable_variant_strings() {
    // PARSE-15: the read-side pretty-printer only formats read statements. The
    // round-trip property test never positively exercised the non-read arms, so
    // a regression that started formatting (or mislabeling) a Mutate/Ddl/Call/
    // Session AST would slip through. Pin each arm's stable variant string.
    for (source, expected_variant) in [
        ("INSERT (:A) FINISH", "MutateStatement"),
        ("CREATE NODE TYPE :Person (name :: STRING)", "DdlStatement"),
        ("CALL selene.health()", "ProcedureCall"),
        ("EXPLAIN RETURN 1", "ExplainStatement"),
        ("START TRANSACTION", "TransactionControl"),
        ("COMMIT", "TransactionControl"),
        ("ROLLBACK", "TransactionControl"),
        ("SESSION SET VALUE $x = 1", "SessionControl"),
        ("SESSION SET TIME ZONE 'UTC'", "SessionControl"),
        ("SESSION RESET", "SessionControl"),
        ("SESSION CLOSE", "SessionControl"),
    ] {
        let parsed = parse(source).unwrap_or_else(|error| panic!("{source} parses: {error:?}"));
        let error = format_read_statement(&parsed)
            .expect_err(&format!("{source} is not a read-side statement"));
        let FormatError::Unsupported { variant } = error else {
            panic!("{source}: expected Unsupported, got {error:?}");
        };
        assert_eq!(variant, expected_variant, "variant string for {source}");
    }
}

#[test]
fn read_side_formatting_is_byte_idempotent() {
    // PARSE-15: structural equality already round-trips, but the FORMATTER's own
    // output should be a fixed point — re-formatting the reparsed AST must
    // produce byte-identical text. This catches a formatter that emits
    // structurally-equivalent-but-textually-drifting output (e.g. fluctuating
    // whitespace or quoting), which structurally_eq would miss.
    for source in [
        "MATCH (n:Person {name: 'Ada'}) RETURN n.name AS name",
        "RETURN (1 + 2) * 3 AS n",
        "RETURN DISTINCT 1 AS \"WITH\"",
        "MATCH (a)-[:KNOWS*1..3]-(b) RETURN b",
        "RETURN n IS TYPED LIST<INT8>",
        "RETURN 1 UNION ALL RETURN 2",
        "MATCH (n) RETURN n NEXT MATCH (m) RETURN m",
    ] {
        let parsed = parse(source).expect("source parses");
        let first = format_read_statement(&parsed).expect("formats once");
        let reparsed = parse(&first).expect("formatted output reparses");
        let second = format_read_statement(&reparsed).expect("formats twice");
        assert_eq!(
            first, second,
            "formatter is not byte-idempotent for {source:?}"
        );
    }
}

#[test]
fn string_literal_escapes_round_trip() {
    // PARSE-19: the formatter's `escape_string` re-encodes a narrower set
    // (\\, \n, \r, \t, '') than the parser's `decode_escape` decodes (which also
    // handles \b \f \u \U \" \`). Round-trip safety across newline/tab/backslash/
    // quote/unicode-control/emoji was never differentially pinned. Build the
    // literal as a runtime Value through the parser, format it, reparse, and
    // assert structural equality so any escaping asymmetry surfaces.
    let bodies = [
        "plain",
        "with\nnewline",
        "tab\there",
        "carriage\rreturn",
        "back\\slash",
        "single'quote",
        "doubled''quote",
        "double\"quote",
        "back`tick",
        "null\u{0000}byte",
        "bell\u{0007}and\u{0008}backspace",
        "form\u{000c}feed",
        "vertical\u{000b}tab",
        "unicode\u{009f}control",
        "emoji \u{1f600}\u{1f680} mix",
        "combining e\u{0301}",
    ];
    for body in bodies {
        let escaped = body.replace('\\', "\\\\").replace('\'', "''");
        let source = format!("RETURN '{escaped}' AS s");
        let parsed = parse(&source).unwrap_or_else(|error| panic!("{body:?} parses: {error:?}"));
        let formatted = format_read_statement(&parsed).expect("formats");
        let reparsed = parse(&formatted).unwrap_or_else(|error| {
            panic!("{body:?} reformatted {formatted:?} reparses: {error:?}")
        });
        assert!(
            structurally_eq(&parsed, &reparsed),
            "string literal {body:?} did not round-trip; formatted as {formatted:?}"
        );
    }
}

fn assert_round_trip(source: &str) {
    let parsed = parse(source).expect("source parses");
    let formatted = format_read_statement(&parsed).expect("read-side AST formats");
    let reparsed = parse(&formatted).expect("formatted source parses");
    assert!(
        structurally_eq(&parsed, &reparsed),
        "formatted source was {formatted:?}"
    );
}

fn is_read_side(statement: &Statement) -> bool {
    matches!(
        statement,
        Statement::Query(_) | Statement::Composite { .. } | Statement::Chained { .. }
    )
}
