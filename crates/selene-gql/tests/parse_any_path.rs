//! Parser coverage for ISO counted `ANY` path search (Feature G016).

use selene_core::feature_register::FeatureId;
use selene_gql::{
    GqlStatus, ParserError, PathMode, PathSelector, PipelineStatement, Statement, feature_walk,
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

#[test]
fn parse_any_path_count_selector() {
    let bare = match_clause("MATCH ANY (a)-[:K]->(b) RETURN b");
    assert_eq!(bare.selector, Some(PathSelector::Any { paths: 1 }));

    let counted = match_clause("MATCH ANY 3 TRAIL PATHS (a)-[:K]->(b) RETURN b");
    assert_eq!(counted.selector, Some(PathSelector::Any { paths: 3 }));
    assert_eq!(counted.path_mode, PathMode::Trail);
    assert!(counted.path_or_paths);
}

#[test]
fn any_path_count_records_g016_only() {
    let observed = feature_walk(&parse("MATCH ANY 3 (a)-[:K]->(b) RETURN b").expect("parse"))
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    assert!(observed.contains(&FeatureId::G016), "{observed:?}");
    assert!(!observed.contains(&FeatureId::G018), "{observed:?}");
    assert!(!observed.contains(&FeatureId::G019), "{observed:?}");
}

#[test]
fn any_zero_count_is_rejected_with_22g0f() {
    let err = parse("MATCH ANY 0 (a)-[:K]->(b) RETURN b").expect_err("ANY 0 rejects");
    assert!(
        matches!(err, ParserError::SyntaxError { .. }),
        "expected SyntaxError, got {err:?}"
    );
    assert_eq!(
        err.gqlstatus(),
        GqlStatus::INVALID_NUMBER_OF_PATHS_OR_GROUPS
    );
    assert_eq!(err.gqlstatus().as_str(), "22G0F");
}

#[test]
fn any_count_cannot_combine_with_shortest() {
    parse("MATCH ANY 2 SHORTEST (a)-[:K]->(b) RETURN b")
        .expect_err("ANY count and SHORTEST are different ISO path searches");
}
