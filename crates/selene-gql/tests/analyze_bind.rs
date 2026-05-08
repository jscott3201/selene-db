//! Analyzer positive binding tests.

use selene_gql::{
    AnalysisError, BindingDeclKind, BindingUseKind, PipelineStatement, Statement, analyze, parse,
};
use selene_testing::analyzed_corpus::load_default_analyzed_corpus;

fn analyze_one(source: &str) -> Result<selene_gql::AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement)
}

#[test]
fn positive_corpus_analyzes_and_resolves_references() {
    let positives = load_default_analyzed_corpus(|source| {
        let statement = parse(source).map_err(|err| err.to_string())?;
        analyze(statement).map_err(|err| err.to_string())
    })
    .expect("positive corpus analyzes");
    assert!(!positives.is_empty());

    for entry in positives {
        for reference in &entry.analyzed.references {
            let declaration = entry
                .analyzed
                .scopes
                .declaration(reference.binding)
                .expect("reference points at declaration");
            assert_eq!(
                declaration.name(),
                reference.name,
                "{} reference name resolves to matching declaration",
                entry.case.path.display()
            );
        }
    }
}

#[test]
fn order_by_alias_resolves_to_projection() {
    let analyzed = analyze_one("MATCH (n) RETURN n.name AS who ORDER BY who").expect("analyzes");
    let alias = analyzed
        .scopes
        .declarations()
        .iter()
        .find(|decl| {
            decl.kind() == BindingDeclKind::ProjectionAlias && decl.name().as_str() == "who"
        })
        .expect("projection alias exists")
        .id();
    assert!(
        analyzed
            .references
            .iter()
            .any(|reference| reference.kind == BindingUseKind::Variable
                && reference.name.as_str() == "who"
                && reference.binding == alias)
    );
}

#[test]
fn explicit_yield_columns_bind_by_visible_name() {
    let analyzed = analyze_one("MATCH (n) CALL pkg.fn(n) YIELD col AS answer RETURN answer")
        .expect("analyzes");
    assert!(analyzed.scopes.declarations().iter().any(|decl| decl.kind()
        == BindingDeclKind::YieldColumn
        && decl.name().as_str() == "answer"));
}

#[test]
fn yield_star_records_wildcard_without_declaring_columns() {
    let analyzed = analyze_one("CALL pkg.fn() YIELD *").expect("analyzes");
    assert_eq!(analyzed.yield_stars.len(), 1);
    assert!(analyzed.scopes.declarations().is_empty());
}

#[test]
fn pattern_reuse_records_reference_not_shadow() {
    let analyzed = analyze_one("MATCH (n), (n) RETURN n").expect("analyzes");
    let node_decls = analyzed
        .scopes
        .declarations()
        .iter()
        .filter(|decl| decl.kind() == BindingDeclKind::NodePattern)
        .count();
    assert_eq!(node_decls, 1);
    assert!(
        analyzed
            .references
            .iter()
            .any(|reference| reference.kind == BindingUseKind::PatternReuse
                && reference.name.as_str() == "n")
    );
}

#[test]
fn with_projection_boundary_hides_pre_with_bindings() {
    let err = analyze_one("MATCH (n) WITH 1 AS x RETURN n").expect_err("n is hidden");
    assert!(matches!(err, AnalysisError::UndefinedReference { .. }));
}

#[test]
fn with_projection_alias_survives_boundary() {
    analyze_one("MATCH (n) WITH n AS kept RETURN kept").expect("alias survives");
}

#[test]
fn top_level_shapes_without_data_bindings_analyze() {
    for source in [
        "CREATE GRAPH IF NOT EXISTS demo",
        "CREATE NODE TYPE :Person (id :: STRING)",
        "START TRANSACTION",
        "COMMIT",
        "ROLLBACK",
    ] {
        analyze_one(source).unwrap_or_else(|err| panic!("{source} should analyze: {err}"));
    }
}

#[test]
fn analyzed_statement_preserves_top_level_shape() {
    let analyzed = analyze_one("MATCH (n) RETURN n").expect("analyzes");
    let selene_gql::AnalyzedStatementKind::Query(query) = analyzed.statement else {
        panic!("expected query");
    };
    assert!(matches!(query.statements[0], PipelineStatement::Match(_)));

    let Statement::Call(_) = parse("CALL pkg.fn()").expect("parses") else {
        panic!("expected call");
    };
}
