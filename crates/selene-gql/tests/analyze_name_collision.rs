//! Broader analyzer same-name collision regressions.

use selene_gql::analyze::PatternElementKind;
use selene_gql::{AnalysisError, BindingDeclKind, EmptyProcedureRegistry, analyze, parse};

fn analyze_one(source: &str) -> Result<selene_gql::AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement, &EmptyProcedureRegistry, None)
}

fn assert_alias_reused_as_node(source: &str, prior_kind: BindingDeclKind) {
    let err = analyze_one(source).expect_err("alias reuse as node pattern rejects");
    assert!(matches!(
        err,
        AnalysisError::AliasReusedAsPatternBinding {
            prior_kind: found,
            new_kind: PatternElementKind::Node,
            ..
        } if found == prior_kind
    ));
}

fn assert_pattern_kind_mismatch(
    source: &str,
    prior: PatternElementKind,
    current: PatternElementKind,
) {
    let err = analyze_one(source).expect_err("cross-kind pattern reuse rejects");
    assert!(matches!(
        err,
        AnalysisError::PatternKindMismatch {
            prior: found_prior,
            current: found_current,
            ..
        } if found_prior == prior && found_current == current
    ));
}

#[test]
fn unwind_alias_same_name_does_not_collide_with_node_pattern() {
    assert_alias_reused_as_node(
        "UNWIND [1] AS x MATCH (x) RETURN x",
        BindingDeclKind::UnwindAlias,
    );
}

#[test]
fn for_position_alias_must_not_shadow_element_alias() {
    let err = analyze_one("FOR x IN [1] WITH ORDINALITY x RETURN x")
        .expect_err("position alias shadows element alias");
    assert!(matches!(err, AnalysisError::Shadow { .. }));
}

#[test]
fn projection_alias_same_name_does_not_collide_with_node_pattern() {
    assert_alias_reused_as_node(
        "MATCH (n) RETURN n AS x NEXT MATCH (x) RETURN x",
        BindingDeclKind::ProjectionAlias,
    );
}

#[test]
fn path_binding_same_name_does_not_collide_with_node_pattern() {
    assert_pattern_kind_mismatch(
        "MATCH p = (a)-[:K]->(b), (p) RETURN p",
        PatternElementKind::Path,
        PatternElementKind::Node,
    );
}

#[test]
fn node_binding_same_name_does_not_collide_with_edge_pattern() {
    assert_pattern_kind_mismatch(
        "MATCH (x), ()-[x]->() RETURN x",
        PatternElementKind::Node,
        PatternElementKind::Edge,
    );
}

#[test]
fn edge_binding_same_name_does_not_collide_with_node_pattern() {
    assert_pattern_kind_mismatch(
        "MATCH ()-[x]->(), (x) RETURN x",
        PatternElementKind::Edge,
        PatternElementKind::Node,
    );
}
