//! Analyzer positive binding tests.

use selene_core::{IStr, intern};
use selene_gql::{
    AnalysisError, BindingDeclKind, BindingUseKind, EmptyProcedureRegistry, GqlType,
    PipelineStatement, ProcedureOutputColumn, ProcedureParameter, ProcedureRegistry, Statement,
    analyze, parse,
};
use selene_testing::analyzed_corpus::load_default_analyzed_gql_corpus;
use selene_testing::{MockProcedureRegistry, default_corpus_registry};

fn analyze_one(source: &str) -> Result<selene_gql::AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement, &EmptyProcedureRegistry)
}

fn analyze_with(
    source: &str,
    registry: &dyn ProcedureRegistry,
) -> Result<selene_gql::AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement, registry)
}

fn pkg_fn_registry(
    parameters: Vec<ProcedureParameter>,
    output_columns: Vec<ProcedureOutputColumn>,
) -> MockProcedureRegistry {
    MockProcedureRegistry::new().with_procedure(
        vec![istr("pkg"), istr("fn")],
        parameters,
        output_columns,
    )
}

fn istr(value: &str) -> IStr {
    intern(value).expect("test strings fit interner")
}

#[test]
fn positive_corpus_analyzes_and_resolves_references() {
    let registry = default_corpus_registry();
    let positives = load_default_analyzed_gql_corpus(&registry).expect("positive corpus analyzes");
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
    let registry = pkg_fn_registry(
        vec![ProcedureParameter {
            name: istr("node"),
            ty: GqlType::NodeRef,
            nullable: false,
        }],
        vec![ProcedureOutputColumn {
            name: istr("col"),
            ty: GqlType::String,
        }],
    );
    let analyzed = analyze_with(
        "MATCH (n) CALL pkg.fn(n) YIELD col AS answer RETURN answer",
        &registry,
    )
    .expect("analyzes");
    assert!(analyzed.scopes.declarations().iter().any(|decl| decl.kind()
        == BindingDeclKind::YieldColumn
        && decl.name().as_str() == "answer"));
}

#[test]
fn yield_star_expands_registered_columns() {
    let registry = pkg_fn_registry(
        Vec::new(),
        vec![
            ProcedureOutputColumn {
                name: istr("first"),
                ty: GqlType::String,
            },
            ProcedureOutputColumn {
                name: istr("second"),
                ty: GqlType::Integer,
            },
        ],
    );
    let analyzed = analyze_with("CALL pkg.fn() YIELD *", &registry).expect("analyzes");
    let yield_names = analyzed
        .scopes
        .declarations()
        .iter()
        .filter(|decl| decl.kind() == BindingDeclKind::YieldColumn)
        .map(|decl| decl.name().as_str())
        .collect::<Vec<_>>();
    assert_eq!(yield_names, ["first", "second"]);
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
fn return_projection_is_permeable_for_ga07_order_by() {
    // GA07: ORDER BY may reference pre-RETURN bindings even when they are
    // not republished as aliases. Codex P1 on PR #25.
    analyze_one("MATCH (n) RETURN n.name AS who ORDER BY n.age").expect("GA07 ordering");
    analyze_one("MATCH (n) RETURN n.name AS who LIMIT 10").expect("LIMIT after RETURN");
}

#[test]
fn return_star_preserves_input_bindings_for_post_return_clauses() {
    // RETURN * does not redeclare aliases; pre-RETURN bindings must stay
    // visible for ORDER BY / LIMIT / OFFSET. Codex P1 on PR #25.
    analyze_one("MATCH (n) RETURN * ORDER BY n.name").expect("RETURN * keeps n visible");
}

#[test]
fn next_chain_threads_bindings_forward() {
    // NEXT consumes the prior block's terminal scope. Codex P1 on PR #25.
    analyze_one("MATCH (n) RETURN n NEXT RETURN n").expect("n flows across NEXT");
}

#[test]
fn mixed_yield_star_binds_explicit_columns() {
    let registry = pkg_fn_registry(
        Vec::new(),
        vec![ProcedureOutputColumn {
            name: istr("result"),
            ty: GqlType::String,
        }],
    );
    let analyzed = analyze_with("CALL pkg.fn() YIELD *, result AS alias", &registry)
        .expect("mixed YIELD analyses");
    assert!(
        analyzed
            .scopes
            .declarations()
            .iter()
            .any(|decl| decl.kind() == BindingDeclKind::YieldColumn
                && decl.name().as_str() == "alias")
    );
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
