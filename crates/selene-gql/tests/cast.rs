//! BRIEF-135a commit 1 + commit 2 acceptance bars — CAST(<expr> AS <type>)
//! parser, analyzer, walker, format, GQLSTATUS, feature-flag, and runtime
//! ISO §22 dispatch matrix coverage.

use selene_core::{GraphId, Value, feature_register::FeatureId};
use selene_gql::{
    AnalysisError, AnalyzedStatement, AnalyzedStatementKind, AnalyzedType, EmptyProcedureRegistry,
    GqlStatus, GqlType, PipelineStatement, ReturnItem, Session, Statement, StatementOutput,
    ValueExpr, analyze, ast::format::format_read_statement, feature_walk, parse,
};
use selene_graph::SharedGraph;

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

// ---------------------------------------------------------------------------
// §F commit 2 bars — runtime CAST evaluator ISO §22 dispatch matrix
// ---------------------------------------------------------------------------

fn execute_first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_500));
    let mut session = Session::new(&graph);
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    let table = match output {
        StatementOutput::Rows(table) => table,
        other => panic!("`{source}` produced no row output: {other:?}"),
    };
    table
        .rows()
        .first()
        .and_then(|row| row.values().first())
        .cloned()
        .unwrap_or_else(|| panic!("`{source}` produced an empty row"))
}

fn execute_first_status(source: &str) -> String {
    let graph = SharedGraph::new(GraphId::new(13_501));
    let mut session = Session::new(&graph);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

fn as_string(value: Value) -> String {
    match value {
        Value::String(istr) => istr.as_str().to_owned(),
        Value::ExternalString(arc) => arc.as_ref().to_owned(),
        other => panic!("expected string, got {other:?}"),
    }
}

#[test]
fn cast_integer_to_string_round_trip() {
    assert_eq!(
        as_string(execute_first_value("RETURN CAST(42 AS STRING) AS v")),
        "42"
    );
}

#[test]
fn cast_string_to_integer_valid_parse() {
    assert_eq!(
        execute_first_value("RETURN CAST('42' AS INTEGER) AS v"),
        Value::Int(42)
    );
}

#[test]
fn cast_string_to_integer_returns_22018() {
    assert_eq!(
        execute_first_status("RETURN CAST('abc' AS INTEGER) AS v"),
        "22018"
    );
}

#[test]
fn cast_float_to_integer_truncates_toward_zero() {
    // ISO §22.4 — truncate toward zero. 3.7 -> 3, -3.7 -> -3.
    assert_eq!(
        execute_first_value("RETURN CAST(3.7 AS INTEGER) AS v"),
        Value::Int(3)
    );
    assert_eq!(
        execute_first_value("RETURN CAST(-3.7 AS INTEGER) AS v"),
        Value::Int(-3)
    );
}

#[test]
fn cast_float_to_integer_overflow_returns_22003() {
    // 1.0e30 is far beyond i64::MAX (~9.2e18); the explicit range check
    // fires before Rust's saturating `as` cast hides the overflow. GQL
    // float-literal grammar requires `digits.digits` (no bare scientific
    // form), so the exponent is on a normalized literal.
    assert_eq!(
        execute_first_status("RETURN CAST(1.0e30 AS INTEGER) AS v"),
        "22003"
    );
}

#[test]
fn cast_float_nan_to_integer_returns_22018() {
    // CAST of NaN to INTEGER has no representable image; ISO §22 emits
    // 22018 (invalid-character-value-for-cast). NaN has no GQL literal,
    // so the integration test threads `f64::NAN` through the runtime via
    // a session parameter binding — exercising the full parse → analyze
    // → plan → execute → evaluator pipeline end-to-end. The inline unit
    // test in `runtime/evaluator/cast.rs::tests::float_nan_to_integer_returns_22018`
    // pins the same branch in isolation.
    let graph = SharedGraph::new(GraphId::new(13_520));
    let mut session = Session::new(&graph);
    session.bind_parameter(
        selene_core::intern("nan").expect("intern parameter name"),
        Value::Float(f64::NAN),
    );
    let status = session
        .execute_source("RETURN CAST($nan AS INTEGER) AS v", &EmptyProcedureRegistry)
        .expect_err("NaN cast must reject at runtime")
        .gqlstatus()
        .as_str()
        .to_owned();
    assert_eq!(status, "22018");
}

#[test]
fn cast_integer_to_float_preserves_value() {
    assert_eq!(
        execute_first_value("RETURN CAST(42 AS FLOAT) AS v"),
        Value::Float(42.0)
    );
}

#[test]
fn cast_float_to_string_round_trip() {
    assert_eq!(
        as_string(execute_first_value("RETURN CAST(3.5 AS STRING) AS v")),
        "3.5"
    );
}

#[test]
fn cast_string_to_float_valid_parse() {
    assert_eq!(
        execute_first_value("RETURN CAST('2.5' AS FLOAT) AS v"),
        Value::Float(2.5)
    );
}

#[test]
fn cast_string_to_float_returns_22018() {
    assert_eq!(
        execute_first_status("RETURN CAST('2.5abc' AS FLOAT) AS v"),
        "22018"
    );
}

#[test]
fn cast_boolean_to_string_lowercase() {
    // ISO §22 — boolean→string yields lowercase "true"/"false".
    assert_eq!(
        as_string(execute_first_value("RETURN CAST(true AS STRING) AS v")),
        "true"
    );
    assert_eq!(
        as_string(execute_first_value("RETURN CAST(false AS STRING) AS v")),
        "false"
    );
}

#[test]
fn cast_string_to_boolean_strict_lowercase_only() {
    // Q3 fold — strict lowercase. "TRUE"/"True"/"YES" all reject as 22018.
    assert_eq!(
        execute_first_value("RETURN CAST('true' AS BOOLEAN) AS v"),
        Value::Bool(true)
    );
    assert_eq!(
        execute_first_value("RETURN CAST('false' AS BOOLEAN) AS v"),
        Value::Bool(false)
    );
    for bad in ["TRUE", "True", "FALSE", "yes", "1"] {
        let source = format!("RETURN CAST('{bad}' AS BOOLEAN) AS v");
        assert_eq!(
            execute_first_status(&source),
            "22018",
            "input `{bad}` must reject as 22018"
        );
    }
}

#[test]
fn cast_boolean_to_integer_zero_or_one() {
    assert_eq!(
        execute_first_value("RETURN CAST(true AS INTEGER) AS v"),
        Value::Int(1)
    );
    assert_eq!(
        execute_first_value("RETURN CAST(false AS INTEGER) AS v"),
        Value::Int(0)
    );
}

#[test]
fn cast_integer_to_boolean_zero_is_false_one_is_true() {
    assert_eq!(
        execute_first_value("RETURN CAST(0 AS BOOLEAN) AS v"),
        Value::Bool(false)
    );
    assert_eq!(
        execute_first_value("RETURN CAST(1 AS BOOLEAN) AS v"),
        Value::Bool(true)
    );
}

#[test]
fn cast_integer_to_boolean_other_returns_22000() {
    // Any integer other than 0/1 has no boolean image. The dispatch matrix
    // returns a generic 22000 data exception (not 22018, which is reserved
    // for STRING parse failures per ISO §22).
    assert_eq!(
        execute_first_status("RETURN CAST(2 AS BOOLEAN) AS v"),
        "22000"
    );
}

#[test]
fn cast_null_to_any_returns_null() {
    // §22 universal — NULL casts to NULL regardless of target.
    assert_eq!(
        execute_first_value("RETURN CAST(NULL AS INTEGER) AS v"),
        Value::Null
    );
    assert_eq!(
        execute_first_value("RETURN CAST(NULL AS STRING) AS v"),
        Value::Null
    );
    assert_eq!(
        execute_first_value("RETURN CAST(NULL AS BOOLEAN) AS v"),
        Value::Null
    );
    assert_eq!(
        execute_first_value("RETURN CAST(NULL AS LIST<INTEGER>) AS v"),
        Value::Null
    );
}

#[test]
fn cast_list_integer_to_string_element_wise() {
    // Q5 — LIST<T> casts element-wise. Source must also be a list.
    let value = execute_first_value("RETURN CAST([1, 2, 3] AS LIST<STRING>) AS v");
    let Value::List(items) = value else {
        panic!("expected list, got {value:?}");
    };
    assert_eq!(items.len(), 3);
    for (idx, item) in items.into_iter().enumerate() {
        assert_eq!(as_string(item), (idx + 1).to_string());
    }
}

#[test]
fn cast_list_list_integer_to_list_list_string() {
    // Nested LIST recursion exercises the stacker-grown path in cast.rs.
    let value = execute_first_value("RETURN CAST([[1, 2], [3, 4]] AS LIST<LIST<STRING>>) AS v");
    let Value::List(outer) = value else {
        panic!("expected outer list, got {value:?}");
    };
    assert_eq!(outer.len(), 2);
    let mut flat = Vec::new();
    for inner in outer {
        let Value::List(items) = inner else {
            panic!("expected inner list, got {inner:?}");
        };
        for item in items {
            flat.push(as_string(item));
        }
    }
    assert_eq!(flat, vec!["1", "2", "3", "4"]);
}

#[test]
fn cast_node_to_anything_returns_42n01() {
    // Source NODE is rejected as 42N01 per ISO §22 (no defined image).
    // Seed the graph with one node so MATCH binds at least one row, then
    // attempt the cast in a second statement on the same session.
    let graph = SharedGraph::new(GraphId::new(13_510));
    let mut session = Session::new(&graph);
    session
        .execute_source("INSERT (:Person)", &EmptyProcedureRegistry)
        .expect("seed insert");
    let status = session
        .execute_source(
            "MATCH (n:Person) RETURN CAST(n AS STRING) AS v",
            &EmptyProcedureRegistry,
        )
        .expect_err("NODE cast rejects")
        .gqlstatus()
        .as_str()
        .to_owned();
    assert_eq!(status, "42N01");
}

#[test]
fn cast_path_to_anything_returns_42n01() {
    // Source PATH / EDGE is rejected per §A.3 (both NODE-family graph
    // elements are 42N01). Seed one edge and try to cast the edge
    // variable, which binds as `Value::EdgeRef` in the executor.
    let graph = SharedGraph::new(GraphId::new(13_511));
    let mut session = Session::new(&graph);
    session
        .execute_source("INSERT (:A)-[:REL]->(:B)", &EmptyProcedureRegistry)
        .expect("seed edge insert");
    let status = session
        .execute_source(
            "MATCH (a:A)-[r:REL]->(b:B) RETURN CAST(r AS STRING) AS v",
            &EmptyProcedureRegistry,
        )
        .expect_err("EDGE cast rejects")
        .gqlstatus()
        .as_str()
        .to_owned();
    assert_eq!(status, "42N01");
}

#[test]
fn cast_record_to_anything_returns_42n01() {
    // Construct a record literal and attempt to cast it.
    let source = "RETURN CAST({a: 1, b: 2} AS STRING) AS v";
    assert_eq!(execute_first_status(source), "42N01");
}

#[test]
fn cast_anything_to_null_returns_42n01() {
    // §A.3 — CAST to GqlType::Null is rejected as 42N01 (cannot cast TO
    // null). The grammar requires SQL-style explicit NULL as a type token;
    // selene-db's type grammar accepts `NULL` per `ast/types.rs:84`.
    let source = "RETURN CAST(1 AS NULL) AS v";
    assert_eq!(execute_first_status(source), "42N01");
}

// ---------------------------------------------------------------------------
// §F commit 3 bars — feature registration + corpus + CHANGELOG
// ---------------------------------------------------------------------------

#[test]
fn feature_ge08_registered_as_supported() {
    use selene_core::feature_register::SUPPORTED_FEATURES;
    assert!(
        SUPPORTED_FEATURES.contains(&FeatureId::GE08),
        "FeatureId::GE08 must be in SUPPORTED_FEATURES; observed {:?}",
        SUPPORTED_FEATURES
            .iter()
            .map(|f| f.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn iso_conformance_ge08_positive_corpus_all_pass() {
    // Walk every GE08-declared positive corpus entry and confirm it
    // parses, analyzes, and surfaces GE08 in the feature-walk output.
    use selene_testing::corpus::{CorpusKind, Expectation, load_default_corpus};

    let cases = load_default_corpus().expect("corpus loads");
    let ge08_cases: Vec<_> = cases
        .iter()
        .filter(|case| {
            case.kind == CorpusKind::Positive
                && case.expectation == Expectation::ParseOk
                && case.declared_features().any(|f| f == FeatureId::GE08)
        })
        .collect();
    assert!(
        ge08_cases.len() >= 5,
        "commit 3 lands at least 5 GE08 positive corpus entries; found {}",
        ge08_cases.len()
    );
    for case in ge08_cases {
        let statement = parse(&case.source)
            .unwrap_or_else(|err| panic!("{} failed to parse: {err:?}", case.path.display()));
        let features = feature_walk(&statement)
            .into_iter()
            .map(|f| f.feature_id)
            .collect::<Vec<_>>();
        assert!(
            features.contains(&FeatureId::GE08),
            "{} did not record GE08; observed {features:?}",
            case.path.display()
        );
        analyze(statement, &EmptyProcedureRegistry, None)
            .unwrap_or_else(|err| panic!("{} failed to analyze: {err:?}", case.path.display()));
    }
}

#[test]
fn changelog_unreleased_added_entry() {
    // Pin the CHANGELOG entry verbatim so a forgotten or relocated
    // entry breaks the build, not the release prep.
    let changelog = include_str!("../../../CHANGELOG.md");
    let unreleased = changelog
        .split("## [Unreleased]")
        .nth(1)
        .expect("CHANGELOG has an [Unreleased] section");
    let added = unreleased
        .split("### ")
        .find(|section| section.starts_with("Added"))
        .expect("[Unreleased] has an `Added` section");
    assert!(
        added.contains("CAST(<expr> AS <type>)"),
        "[Unreleased].Added must mention `CAST(<expr> AS <type>)`; observed first 400 chars: {}",
        &added[..added.len().min(400)]
    );
    assert!(
        added.contains("GE08"),
        "[Unreleased].Added must mention feature `GE08`"
    );
    assert!(
        added.contains("22018"),
        "[Unreleased].Added must mention the new GQLSTATUS `22018`"
    );
}
