//! Read-side AST pretty-printer round-trip property tests.

use proptest::test_runner::TestRunner;
use selene_gql::{Statement, ast::format_read_statement, ast::structurally_eq, parse};
use selene_testing::corpus::{CorpusKind, Expectation, load_default_corpus};

#[test]
fn representative_read_shapes_round_trip() {
    for source in [
        "MATCH (n:Person {name: 'Ada'}) RETURN n.name AS name",
        "RETURN (1 + 2) * 3 AS n",
        "RETURN count(*) AS c",
        "RETURN CASE WHEN n.age > 10 THEN 'old' ELSE 'new' END AS bucket",
        "MATCH ANY SHORTEST (a)-[:K]->(b) RETURN b",
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
