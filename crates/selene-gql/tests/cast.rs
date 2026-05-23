//! BRIEF-135a commit 1 acceptance bars — CAST(<expr> AS <type>) parser,
//! analyzer, walker, format, GQLSTATUS, and feature-flag coverage. Runtime
//! evaluation is stubbed at `FeatureNotInV1_1` in this commit; commit 2
//! lands the §22 dispatch matrix and adds the runtime test suite.

use selene_core::feature_register::FeatureId;
use selene_gql::{
    AnalysisError, AnalyzedStatement, AnalyzedStatementKind, AnalyzedType, EmptyProcedureRegistry,
    GqlStatus, GqlType, PipelineStatement, ReturnItem, Statement, ValueExpr, analyze,
    ast::format::format_read_statement, feature_walk, parse,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_or_panic(source: &str) -> Statement {
    parse(source).unwrap_or_else(|err| panic!("parse failed for `{source}`: {err:?}"))
}

fn analyze_one(source: &str) -> Result<AnalyzedStatement, AnalysisError> {
    analyze(parse_or_panic(source), &EmptyProcedureRegistry, None)
}

fn analyze_or_panic(source: &str) -> AnalyzedStatement {
    analyze_one(source).unwrap_or_else(|err| panic!("analyze failed for `{source}`: {err:?}"))
}

fn return_items(analyzed: &AnalyzedStatement) -> &[ReturnItem] {
    let AnalyzedStatementKind::Query(query) = &analyzed.statement else {
        panic!("expected query statement");
    };
    query
        .statements
        .iter()
        .find_map(|statement| match statement {
            PipelineStatement::Return(clause) => Some(clause.items.as_slice()),
            _ => None,
        })
        .expect("RETURN clause exists")
}

fn first_return_item_type(analyzed: &AnalyzedStatement) -> AnalyzedType {
    let item = return_items(analyzed)
        .first()
        .expect("at least one RETURN item");
    let id = analyzed
        .expr_ids
        .get(&item.expr)
        .expect("RETURN item has an ExprId");
    analyzed.expr_types.get(id).clone()
}

fn first_return_item_expr(analyzed: &AnalyzedStatement) -> &ValueExpr {
    &return_items(analyzed)
        .first()
        .expect("at least one RETURN item")
        .expr
}

// ---------------------------------------------------------------------------
// §F commit 1 bars
// ---------------------------------------------------------------------------

#[test]
fn parser_accepts_cast_integer_to_string() {
    let stmt = parse_or_panic("RETURN CAST(1 AS STRING) AS s");
    // Confirm the parsed AST contains a Cast node, not a function-call lookalike.
    let analyzed = analyze(stmt, &EmptyProcedureRegistry, None).expect("analyzes");
    assert!(
        matches!(first_return_item_expr(&analyzed), ValueExpr::Cast { .. }),
        "expected ValueExpr::Cast, got {:?}",
        first_return_item_expr(&analyzed)
    );
}

#[test]
fn parser_accepts_cast_string_to_integer() {
    let analyzed = analyze_or_panic("RETURN CAST('42' AS INTEGER) AS n");
    assert!(matches!(
        first_return_item_expr(&analyzed),
        ValueExpr::Cast { .. }
    ));
}

#[test]
fn parser_accepts_cast_with_nested_expression() {
    // Outer CAST receives a sub-expression that already contains a CAST;
    // exercises the recursive `bind_value_expr` path through the variant.
    let analyzed = analyze_or_panic("RETURN CAST(CAST(1 AS STRING) AS INTEGER) AS n");
    let ValueExpr::Cast { value, .. } = first_return_item_expr(&analyzed) else {
        panic!("expected outer CAST");
    };
    assert!(
        matches!(value.as_ref(), ValueExpr::Cast { .. }),
        "expected nested CAST as inner value, got {value:?}",
    );
}

#[test]
fn parser_accepts_cast_with_list_target_type() {
    let analyzed = analyze_or_panic("RETURN CAST([1, 2, 3] AS LIST<INTEGER>) AS l");
    let ValueExpr::Cast { target_type, .. } = first_return_item_expr(&analyzed) else {
        panic!("expected CAST node");
    };
    assert!(
        matches!(target_type.as_ref(), GqlType::List(_)),
        "expected LIST target type, got {target_type:?}",
    );
}

#[test]
fn parser_rejects_cast_with_invalid_grammar() {
    // No `AS` keyword - the cast_expr rule fails to match and pest reports a
    // syntax error.
    let err = parse("RETURN CAST(1 STRING) AS x").expect_err("invalid CAST is rejected");
    let rendered = format!("{err:?}");
    assert!(
        !rendered.is_empty(),
        "expected non-empty parser error for invalid CAST grammar"
    );
}

#[test]
fn analyze_bind_cast_returns_target_type_integer() {
    let analyzed = analyze_or_panic("RETURN CAST('42' AS INTEGER) AS n");
    let ty = first_return_item_type(&analyzed);
    assert_eq!(ty, AnalyzedType::Resolved(GqlType::Integer));
}

#[test]
fn analyze_bind_cast_returns_target_type_string() {
    let analyzed = analyze_or_panic("RETURN CAST(1 AS STRING) AS s");
    let ty = first_return_item_type(&analyzed);
    assert_eq!(ty, AnalyzedType::Resolved(GqlType::String));
}

#[test]
fn analyze_bind_cast_preserves_nullability() {
    // ISO §22 explicit casts propagate NULL → NULL; the analyzer reports the
    // declared target type regardless of source-expression nullability (NULL
    // handling is a runtime concern). This test pins the analyzer contract:
    // CAST(NULL AS INTEGER) statically types as INTEGER, not Dynamic.
    let analyzed = analyze_or_panic("RETURN CAST(NULL AS INTEGER) AS n");
    let ty = first_return_item_type(&analyzed);
    assert_eq!(ty, AnalyzedType::Resolved(GqlType::Integer));
}

#[test]
fn analyze_hash_distinguishes_cast_integer_vs_float_target() {
    // F6 fold: the structural hash MUST hash `target_type` so that two CAST
    // nodes differing only in declared target type don't dedup against each
    // other in the ExprIdLookup. The contract is observable through
    // `expr_ids.len()`: parse two distinct CASTs in the same statement; both
    // get distinct ExprIds.
    let analyzed = analyze_or_panic(
        "RETURN CAST(1 AS INTEGER) AS i, CAST(1 AS FLOAT) AS f, CAST(1 AS STRING) AS s",
    );
    let items = return_items(&analyzed);
    assert_eq!(items.len(), 3, "three RETURN projections");
    let mut ids = std::collections::HashSet::new();
    for item in items {
        let id = analyzed
            .expr_ids
            .get(&item.expr)
            .expect("each CAST projection has an ExprId");
        assert!(
            ids.insert(id),
            "CAST projections with different target_type must hash to distinct ExprIds"
        );
    }
}

#[test]
fn flagger_records_ge08_for_cast_expression() {
    // F7 fold: every parsed `CAST` records FeatureId::GE08 in the
    // feature-walk output. The current GE08 SUPPORTED flip lands in commit 3,
    // so until then `analyze` may surface FeatureNotSupported; this test is
    // therefore phrased to walk the parsed Statement directly.
    let stmt = parse_or_panic("RETURN CAST(1 AS STRING) AS s");
    let features = feature_walk(&stmt)
        .into_iter()
        .map(|f| f.feature_id)
        .collect::<Vec<_>>();
    assert!(
        features.contains(&FeatureId::GE08),
        "CAST must record GE08, observed {features:?}"
    );
}

#[test]
fn walker_audit_16_sites_extended() {
    // F4 fold: 16 walker sites across the analyzer / planner / optimizer /
    // renderer / parser must include the new Cast variant. Each compile-error
    // failure caught by the `cargo build` gate proves a missing arm. This
    // smoke-level integration test confirms the load-bearing end-to-end
    // pipeline (parse → analyze → success) handles Cast everywhere it
    // matters; the exhaustive non-`_` matches enforce site coverage at
    // compile time. The 16th site is `parser/many.rs::rebase_value` (Stage 0
    // F4 missed it; the canonical count is 16, not 15).
    //
    // Nested + LIST + parameter + binary-op-over-cast exercise all the
    // recursive walker paths flagged in §A.4.
    let sources = [
        "RETURN CAST(1 AS STRING) AS a",
        "RETURN CAST(CAST(1 AS STRING) AS INTEGER) AS a",
        "RETURN CAST($p AS INTEGER) AS a",
        "RETURN CAST(1 AS INTEGER) + CAST(2 AS INTEGER) AS a",
        "RETURN CAST([1, 2, 3] AS LIST<INTEGER>) AS a",
    ];
    for source in sources {
        analyze_one(source)
            .unwrap_or_else(|err| panic!("walker audit failed for `{source}`: {err:?}"));
    }
}

#[test]
fn format_cast_round_trips_via_sibling_module() {
    // F3 fold: ast/format/cast.rs lives as a sibling module to keep
    // ast/format.rs under the 700-LOC cap. The contract is that the
    // formatter emits a `CAST(... AS ...)` round-trippable form via the
    // public `format_read_statement` path; reparsing yields a Cast node.
    let source = "RETURN CAST(1 AS STRING) AS s";
    let stmt = parse_or_panic(source);
    let rendered = format_read_statement(&stmt).expect("formats");
    assert!(
        rendered.contains("CAST"),
        "rendered form must contain `CAST`, got `{rendered}`"
    );
    assert!(
        rendered.contains(" AS "),
        "rendered form must contain ` AS `, got `{rendered}`"
    );
    assert!(
        rendered.contains("STRING"),
        "rendered form must contain target type `STRING`, got `{rendered}`"
    );
    // Re-parse the rendered form and confirm the outer RETURN item is still
    // a CAST. This pins both the format direction and parser round-trip.
    let reanalyzed = analyze_or_panic(&rendered);
    assert!(matches!(
        first_return_item_expr(&reanalyzed),
        ValueExpr::Cast { .. }
    ));
}

#[test]
fn gqlstatus_22018_invalid_character_value_for_cast_registered() {
    // F1 fold: GQLSTATUS 22018 is wired through both the canonical Table 8
    // map in selene-core and the `GqlStatus::INVALID_CHARACTER_VALUE_FOR_CAST`
    // const in selene-gql.
    let status = GqlStatus::INVALID_CHARACTER_VALUE_FOR_CAST;
    assert_eq!(status.as_str(), "22018");
    assert_eq!(&status.class(), b"22");
    // Round-trip lookup through the canonical Table 8 name registry.
    let name =
        selene_core::gqlstatus::gqlstatus_name("22018").expect("22018 is registered in Table 8");
    assert_eq!(name, "invalid-character-value-for-cast");
}
