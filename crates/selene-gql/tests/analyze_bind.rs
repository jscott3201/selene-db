//! Analyzer positive binding tests.

use selene_core::DbString;
use selene_gql::{
    AnalysisError, BindingDeclKind, BindingUseKind, EmptyProcedureRegistry, GqlType,
    PipelineStatement, ProcedureOutputColumn, ProcedureParameter, ProcedureRegistry, Statement,
    analyze, parse,
};
use selene_testing::analyzed_corpus::load_default_analyzed_gql_corpus;
use selene_testing::{MockProcedureRegistry, default_corpus_registry};

fn analyze_one(source: &str) -> Result<selene_gql::AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement, &EmptyProcedureRegistry, None)
}

fn analyze_with(
    source: &str,
    registry: &dyn ProcedureRegistry,
) -> Result<selene_gql::AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement, registry, None)
}

fn pkg_fn_registry(
    parameters: Vec<ProcedureParameter>,
    output_columns: Vec<ProcedureOutputColumn>,
) -> MockProcedureRegistry {
    MockProcedureRegistry::new().with_procedure(
        vec![db_string("pkg"), db_string("fn")],
        parameters,
        output_columns,
    )
}

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test strings fit DB string cap")
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
        vec![ProcedureParameter::new(
            db_string("node"),
            GqlType::NodeRef,
            false,
        )],
        vec![ProcedureOutputColumn::new(
            db_string("col"),
            GqlType::String,
        )],
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
            ProcedureOutputColumn::new(db_string("first"), GqlType::String),
            ProcedureOutputColumn::new(db_string("second"), GqlType::Integer),
        ],
    );
    let analyzed = analyze_with("CALL pkg.fn() YIELD *", &registry).expect("analyzes");
    let yield_names = analyzed
        .scopes
        .declarations()
        .iter()
        .filter(|decl| decl.kind() == BindingDeclKind::YieldColumn)
        .map(|decl| decl.name().as_str().to_owned())
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
fn exists_subquery_does_not_refine_outer_label_expr() {
    let analyzed =
        analyze_one("MATCH (n) WHERE EXISTS { MATCH (n:Person) } RETURN n").expect("analyzes");
    let outer = analyzed
        .scopes
        .declarations()
        .iter()
        .find(|decl| decl.kind() == BindingDeclKind::NodePattern && decl.name().as_str() == "n")
        .expect("outer n declaration exists");

    assert!(
        outer.label_expr().is_none(),
        "outer n must not inherit inner EXISTS label refinement"
    );
}

#[test]
fn analyzer_rejects_let_alias_reused_as_node_pattern() {
    let err = analyze_one("LET x = 1 MATCH (x) RETURN x")
        .expect_err("value alias cannot be reused as a node pattern");
    assert!(matches!(
        err,
        AnalysisError::AliasReusedAsPatternBinding { .. }
    ));
}

#[test]
fn unbounded_quantifier_without_iso_gate_rejects_42001() {
    for source in [
        "MATCH (a)-[:K*]->(b) RETURN b",
        "MATCH ALL (a)-[:K+]->(b) RETURN b",
        // An explicit bare WALK (no selector, no DIFFERENT EDGES) is still
        // rejected — the FU-2 unbounded-shortest TRAIL downshift is a planner-only
        // change and must not relax this analyzer gate.
        "MATCH WALK (a)-[:K+]->(b) RETURN b",
    ] {
        let err = analyze_one(source).expect_err("unbounded quantifier requires a gate");
        assert!(matches!(err, AnalysisError::UnboundedRequiresGate { .. }));
        assert_eq!(err.gqlstatus().as_str(), "42001");
    }
}

#[test]
fn unbounded_quantifier_with_restrictive_or_selective_gate_analyzes() {
    for source in [
        "MATCH TRAIL (a)-[:K*]->(b) RETURN b",
        "MATCH ANY (a)-[:K+]->(b) RETURN b",
        "MATCH ALL SHORTEST (a)-[:K{2,}]->(b) RETURN b",
        // ISO §16.6 SR4: the counted shortest path (G019) and counted shortest
        // group (G020) prefixes are selective, so they also gate an unbounded
        // variable-length pattern — the primary use of a shortest selector.
        "MATCH SHORTEST 3 (a)-[:K*]->(b) RETURN b",
        "MATCH SHORTEST 2 GROUPS (a)-[:K+]->(b) RETURN b",
    ] {
        analyze_one(source).unwrap_or_else(|err| panic!("{source} should analyze: {err}"));
    }
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
fn order_by_rejects_value_subquery_sort_key() {
    let err = analyze_one("RETURN 1 AS n ORDER BY VALUE { RETURN 1 LIMIT 1 }")
        .expect_err("sort key cannot contain a nested query specification");
    assert!(matches!(
        err,
        AnalysisError::SortKeyContainsNestedQuery { .. }
    ));
    assert_eq!(err.gqlstatus().as_str(), "42001");

    let err = analyze_one("RETURN 1 AS n ORDER BY 1 + VALUE { RETURN 1 LIMIT 1 }")
        .expect_err("nested value query is rejected inside a larger sort expression");
    assert!(matches!(
        err,
        AnalysisError::SortKeyContainsNestedQuery { .. }
    ));

    analyze_one("RETURN TRUE AS ok ORDER BY EXISTS { MATCH (n) }")
        .expect("EXISTS predicate is not a nested query specification");
}

#[test]
fn order_by_rejects_aggregate_sort_key_without_grouped_aggregate_return() {
    for source in [
        "RETURN 1 AS n ORDER BY count(*)",
        "FOR x IN [1, 2] RETURN sum(x) AS s ORDER BY count(*)",
        "FOR x IN [1, 2] RETURN x AS x GROUP BY x ORDER BY count(*)",
    ] {
        let err = analyze_one(source)
            .expect_err("aggregate sort key requires grouped aggregate RETURN context");
        assert!(
            matches!(err, AnalysisError::SortKeyContainsAggregate { .. }),
            "{source} should reject with SortKeyContainsAggregate, got {err:?}"
        );
        assert_eq!(err.gqlstatus().as_str(), "42001");
    }
}

#[test]
fn order_by_allows_aggregate_sort_key_with_grouped_aggregate_return() {
    analyze_one("FOR x IN [1, 2] RETURN x AS x, count(*) AS c GROUP BY x ORDER BY count(*)")
        .expect("grouped aggregate RETURN context may sort by aggregate function");
}

#[test]
fn group_by_rejects_ungrouped_nonaggregate_return_items() {
    for source in [
        "FOR x IN [1, 2] RETURN x AS x, x + 1 AS y GROUP BY x",
        "FOR x IN [1, 2] RETURN 1 AS one, count(*) AS c GROUP BY ()",
        "FOR x IN [1, 2] WITH x AS x, x + 1 AS y GROUP BY x RETURN x",
    ] {
        let err =
            analyze_one(source).expect_err("non-aggregate projection item must be a GROUP BY key");
        assert!(
            matches!(err, AnalysisError::GroupedProjectionItemNotGrouped { .. }),
            "{source} should reject with GroupedProjectionItemNotGrouped, got {err:?}"
        );
        assert_eq!(err.gqlstatus().as_str(), "42001");
    }
}

#[test]
fn group_by_allows_grouped_and_aggregate_return_items() {
    analyze_one("FOR x IN [1, 2] RETURN x AS x GROUP BY x")
        .expect("grouping key projection is legal without aggregate");
    analyze_one("FOR x IN [1, 2] RETURN x + 1 AS y, count(*) AS c GROUP BY x + 1")
        .expect("projected expression may match the grouping key");
    analyze_one(
        "FOR x IN ['aa', 'bbb'] RETURN char_length(x) AS l, count(*) AS c GROUP BY char_length(x)",
    )
    .expect("span-insensitive grouped expression matching keeps scalar grouping legal");
}

#[test]
fn return_star_preserves_input_bindings_for_post_return_clauses() {
    // RETURN * does not redeclare aliases; pre-RETURN bindings must stay
    // visible for ORDER BY / LIMIT / OFFSET. Codex P1 on PR #25.
    analyze_one("MATCH (n) RETURN * ORDER BY n.name").expect("RETURN * keeps n visible");
}

#[test]
fn return_star_rejects_unit_input() {
    let err = analyze_one("RETURN *").expect_err("unit input has no bindings to expand");
    assert!(matches!(err, AnalysisError::ReturnStarRequiresInput { .. }));
    assert_eq!(err.gqlstatus().as_str(), "42001");

    analyze_one("MATCH (n) RETURN *").expect("RETURN * expands MATCH binding");
    analyze_one("MATCH (n) WITH n AS x RETURN *").expect("RETURN * expands WITH binding");
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
        vec![ProcedureOutputColumn::new(
            db_string("result"),
            GqlType::String,
        )],
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

// ---------------------------------------------------------------------------
// ISO §14.10 SR 4)c)i)2)A)III defines ORDER_REFS in three cases, and SR IV makes
// membership mandatory. The plain case is permissive and the planner carries the
// binding across the projection; the other two close the set, and a reference
// outside it has to be rejected rather than silently evaluate to NULL.
// ---------------------------------------------------------------------------

#[test]
fn order_by_accepts_a_discarded_binding_under_a_plain_return() {
    for source in [
        "MATCH (n:Person) RETURN n.name AS name ORDER BY n.score",
        "MATCH (n:Person) RETURN n.name AS name ORDER BY n.score + 0 DESC",
        "MATCH (n:Person) RETURN 1 AS one ORDER BY n.score, n.name",
    ] {
        analyze_one(source)
            .unwrap_or_else(|err| panic!("{source} is legal under GA07, got {err:?}"));
    }
}

#[test]
fn order_by_rejects_a_discarded_binding_under_distinct_or_aggregation() {
    for source in [
        // DISTINCT: ORDER_REFS is RETURN_IDENTIFIERS alone.
        "MATCH (n:Person) RETURN DISTINCT n.name AS name ORDER BY n.score",
        // An aggregate return item with no GROUP BY: likewise.
        "MATCH (n:Person) RETURN count(*) AS c ORDER BY n.score",
        // GROUP BY: ORDER_REFS adds the grouping keys, and `n` is not one.
        "FOR x IN [1, 2] RETURN x AS x GROUP BY x ORDER BY y",
        // SR IV applies to every sort key, not just the first: the leading term
        // is in scope and only the second one escapes it.
        "MATCH (n:Person) RETURN DISTINCT n.name AS name ORDER BY name, n.score",
        "MATCH (n:Person) RETURN count(*) AS c ORDER BY c DESC, n.score",
    ] {
        let err = analyze_one(source)
            .expect_err("a projection that discards the row cannot be ordered by it");
        assert!(
            matches!(err, AnalysisError::SortKeyReferenceNotInScope { .. }),
            "{source} should reject with SortKeyReferenceNotInScope, got {err:?}"
        );
        assert_eq!(err.gqlstatus().as_str(), "42001");
    }
}

#[test]
fn order_by_accepts_grouping_keys_and_output_columns_under_aggregation() {
    for source in [
        // A grouping key is in ORDER_REFS even when it is not projected.
        "FOR x IN [1, 2] RETURN count(*) AS c GROUP BY x ORDER BY x",
        // SR III case 2 is unconditional on DISTINCT: a GROUP BY clause keeps
        // its grouping keys in ORDER_REFS even under a set quantifier.
        "FOR x IN [1, 2] RETURN DISTINCT count(*) AS c GROUP BY x ORDER BY x",
        // Every sort key is checked, so a multi-term list of in-scope names
        // must still be accepted.
        "MATCH (n:Person) RETURN DISTINCT n.name AS name, n.score AS score \
         ORDER BY score DESC, name",
        // Output columns are always in ORDER_REFS.
        "MATCH (n:Person) RETURN DISTINCT n.name AS name ORDER BY name",
        "MATCH (n:Person) RETURN count(*) AS c ORDER BY c",
    ] {
        analyze_one(source)
            .unwrap_or_else(|err| panic!("{source} orders by an in-scope name, got {err:?}"));
    }
}

/// `RETURN *` keeps the whole input row, so the closing rules do not apply.
///
/// The `DISTINCT` case is the one that makes the star check load-bearing: a
/// star projection has no return items, so without the early return it would
/// fall through to `Closed([])` and reject every sort key.
#[test]
fn order_by_accepts_any_incoming_binding_under_return_star() {
    for source in [
        "MATCH (n:Person) RETURN * ORDER BY n.score",
        "MATCH (n:Person) RETURN DISTINCT * ORDER BY n.score",
    ] {
        analyze_one(source)
            .unwrap_or_else(|err| panic!("{source} keeps every incoming column, got {err:?}"));
    }
}
