//! Analyzer negative binding tests.

use selene_core::db_string;
use selene_gql::{
    AnalysisError, EmptyProcedureRegistry, GqlType, ProcedureOutputColumn, analyze, parse,
};
use selene_testing::MockProcedureRegistry;

fn analyze_one(source: &str) -> Result<selene_gql::AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement, &EmptyProcedureRegistry, None)
}

#[test]
fn undefined_variable_in_return() {
    let err = analyze_one("RETURN x").expect_err("x is undefined");
    assert!(matches!(err, AnalysisError::UndefinedReference { .. }));
}

#[test]
fn undefined_variable_in_match_where() {
    let err = analyze_one("MATCH (n) WHERE x = 1 RETURN n").expect_err("x is undefined");
    assert!(matches!(err, AnalysisError::UndefinedReference { .. }));
}

#[test]
fn shadow_in_let() {
    let err = analyze_one("MATCH (n) LET n = 1 RETURN n").expect_err("LET shadows n");
    assert!(matches!(err, AnalysisError::Shadow { .. }));
}

#[test]
fn shadow_across_match_and_unwind() {
    let err = analyze_one("MATCH (n) UNWIND [1] AS n RETURN n").expect_err("UNWIND shadows n");
    assert!(matches!(err, AnalysisError::Shadow { .. }));
}

#[test]
fn order_by_undefined_alias_errors() {
    let err = analyze_one("MATCH (n) RETURN n.name AS who ORDER BY whom")
        .expect_err("whom is undefined after projection");
    assert!(matches!(err, AnalysisError::UndefinedReference { .. }));
}

#[test]
fn exists_subquery_uses_outer_scope_but_keeps_inner_scope_local() {
    analyze_one("MATCH (n) WHERE EXISTS { MATCH (m)-[:K]->(n) } RETURN n")
        .expect("outer n reachable in EXISTS");
    let err = analyze_one("MATCH (n) WHERE EXISTS { MATCH (m)-[:K]->(n) } RETURN m")
        .expect_err("inner m is not visible outside EXISTS");
    assert!(matches!(err, AnalysisError::UndefinedReference { .. }));
}

#[test]
fn case_branch_uses_outer_scope() {
    analyze_one("MATCH (n) RETURN CASE WHEN n.age > 18 THEN n.name ELSE 'minor' END AS label")
        .expect("CASE branch can use enclosing n");
}

#[test]
fn unknown_name_after_yield_star_expansion_errors() {
    let registry = MockProcedureRegistry::new().with_procedure(
        vec![db_string("pkg").unwrap(), db_string("fn").unwrap()],
        Vec::new(),
        vec![ProcedureOutputColumn::new(
            db_string("out").unwrap(),
            GqlType::String,
        )],
    );
    let statement = parse("MATCH (n) CALL pkg.fn() YIELD * RETURN col").expect("test input parses");
    let err = analyze(statement, &registry, None).expect_err("col not bound");
    assert!(matches!(err, AnalysisError::UndefinedReference { .. }));
}

#[test]
fn cross_kind_pattern_reuse_is_rejected() {
    // Reusing a node variable as an edge variable (or vice versa) must be a
    // semantic error, not a silent alias. Codex P2 on PR #25.
    let err = analyze_one("MATCH (n) MATCH (a)-[n]->(b) RETURN a")
        .expect_err("n cannot be reused as an edge");
    assert!(matches!(err, AnalysisError::PatternKindMismatch { .. }));
}
