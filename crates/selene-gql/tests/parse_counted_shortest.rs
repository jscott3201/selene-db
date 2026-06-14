//! Parser AST-shape tests for the ISO counted shortest path/group selectors
//! (Features G019/G020, ISO/IEC 39075:2024 §16.6).

use selene_gql::{
    GqlStatus, ParserError, PathMode, PathSelector, PipelineStatement, Statement,
    ast::{format_read_statement, structurally_eq},
    parse,
};

fn match_clause(source: &str) -> selene_gql::MatchClause {
    let Statement::Query(query) = parse(source).expect("parse succeeds") else {
        panic!("expected query statement");
    };
    let PipelineStatement::Match(match_clause) = &query.statements[0] else {
        panic!("expected MATCH");
    };
    match_clause.clone()
}

fn match_selector(source: &str) -> Option<PathSelector> {
    match_clause(source).selector
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
fn counted_shortest_path_mode_precedes_group_discriminator() {
    // ISO §16.6 counted forms place <path mode> before optional PATH/PATHS and
    // before GROUP/GROUPS. Pin both counted-path and counted-group AST shapes so
    // the flattened MATCH grammar cannot silently drift back to GROUPS TRAIL.
    let counted_path = match_clause("MATCH SHORTEST 3 TRAIL PATHS (a)-[:K]->(b) RETURN b");
    assert_eq!(
        counted_path.selector,
        Some(PathSelector::CountedShortest { paths: 3 })
    );
    assert_eq!(counted_path.path_mode, PathMode::Trail);
    assert!(counted_path.path_or_paths);

    let counted_group = match_clause("MATCH SHORTEST 2 TRAIL PATHS GROUPS (a)-[:K]->(b) RETURN b");
    assert_eq!(
        counted_group.selector,
        Some(PathSelector::CountedShortestGroup { groups: 2 })
    );
    assert_eq!(counted_group.path_mode, PathMode::Trail);
    assert!(counted_group.path_or_paths);

    let default_group = match_clause("MATCH SHORTEST ACYCLIC GROUP (a)-[:K]->(b) RETURN b");
    assert_eq!(
        default_group.selector,
        Some(PathSelector::CountedShortestGroup { groups: 1 })
    );
    assert_eq!(default_group.path_mode, PathMode::Acyclic);
}

#[test]
fn counted_group_path_mode_formats_in_iso_order() {
    let source = "MATCH SHORTEST 2 TRAIL PATHS GROUPS (a)-[:K]->(b) RETURN b";
    let parsed = parse(source).expect("source parses");
    let formatted = format_read_statement(&parsed).expect("read-side AST formats");
    assert_eq!(
        formatted,
        "MATCH SHORTEST 2 TRAIL PATHS GROUPS (a)-[:K]->(b)\nRETURN b"
    );
    let reparsed = parse(&formatted).expect("formatted source parses");
    assert!(structurally_eq(&parsed, &reparsed));
}

#[test]
fn counted_group_rejects_path_mode_after_group_discriminator() {
    for source in [
        "MATCH SHORTEST 2 GROUPS TRAIL (a)-[:K]->(b) RETURN b",
        "MATCH SHORTEST 2 PATHS GROUPS TRAIL (a)-[:K]->(b) RETURN b",
    ] {
        let err = parse(source).expect_err("path mode after GROUPS must reject");
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "expected SyntaxError for {source:?}, got {err:?}"
        );
    }
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
