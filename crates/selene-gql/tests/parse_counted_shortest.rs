//! Parser AST-shape tests for the ISO counted shortest path/group selectors
//! (Features G019/G020, ISO/IEC 39075:2024 §16.6).

use selene_gql::{GqlStatus, ParserError, PathSelector, PipelineStatement, Statement, parse};

fn match_selector(source: &str) -> Option<PathSelector> {
    let Statement::Query(query) = parse(source).expect("parse succeeds") else {
        panic!("expected query statement");
    };
    let PipelineStatement::Match(match_clause) = &query.statements[0] else {
        panic!("expected MATCH");
    };
    match_clause.selector
}

#[test]
fn parse_counted_shortest_path_and_group_selectors() {
    // ISO 39075:2024 §16.6: SHORTEST N is the counted path search (G019);
    // SHORTEST [N] GROUP[S] is the counted group search (G020). A bare
    // GROUP/GROUPS defaults the count to 1 per §16.6 SR2b.
    assert_eq!(
        match_selector("MATCH SHORTEST 3 (a)-[:K]->(b) RETURN b"),
        Some(PathSelector::CountedShortest { paths: 3 })
    );
    assert_eq!(
        match_selector("MATCH SHORTEST 2 GROUPS (a)-[:K]->(b) RETURN b"),
        Some(PathSelector::CountedShortestGroup { groups: 2 })
    );
    assert_eq!(
        match_selector("MATCH SHORTEST GROUP (a)-[:K]->(b) RETURN b"),
        Some(PathSelector::CountedShortestGroup { groups: 1 })
    );
    assert_eq!(
        match_selector("MATCH SHORTEST GROUPS (a)-[:K]->(b) RETURN b"),
        Some(PathSelector::CountedShortestGroup { groups: 1 })
    );
    assert_eq!(
        match_selector("MATCH SHORTEST 1 GROUP (a)-[:K]->(b) RETURN b"),
        Some(PathSelector::CountedShortestGroup { groups: 1 })
    );
}

#[test]
fn counted_shortest_zero_count_is_rejected_with_22g0f() {
    // ISO 39075:2024 §16.6 SR2bii: a written literal count SHALL be positive.
    // A literal 0 is a static violation; reject it with GQLSTATUS 22G0F (invalid
    // number of paths or groups) rather than silently accepting.
    for source in [
        "MATCH SHORTEST 0 (a)-[:K]->(b) RETURN b",
        "MATCH SHORTEST 0 GROUPS (a)-[:K]->(b) RETURN b",
        // singular GROUP must reject identically to plural GROUPS
        "MATCH SHORTEST 0 GROUP (a)-[:K]->(b) RETURN b",
    ] {
        let err = parse(source).expect_err("SHORTEST 0 must be rejected");
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "expected SyntaxError for {source:?}, got {err:?}"
        );
        assert_eq!(
            err.gqlstatus(),
            GqlStatus::INVALID_NUMBER_OF_PATHS_OR_GROUPS,
            "{source:?} must report 22G0F"
        );
        assert_eq!(err.gqlstatus().as_str(), "22G0F", "{source:?}");
    }
}

#[test]
fn bare_shortest_without_count_or_group_does_not_parse() {
    // ISO 39075:2024 §16.6: a bare `SHORTEST` is neither a counted path (needs a
    // number) nor a counted group (needs GROUP[S]); both counted_shortest_tail
    // alternatives fail, so the selector cannot parse.
    let err =
        parse("MATCH SHORTEST (a)-[:K]->(b) RETURN b").expect_err("bare SHORTEST must not parse");
    assert!(
        matches!(err, ParserError::SyntaxError { .. }),
        "expected SyntaxError for bare SHORTEST, got {err:?}"
    );
}
