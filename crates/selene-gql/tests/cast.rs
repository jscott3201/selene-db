//! BRIEF-135a commit 1 + commit 2 acceptance bars — CAST(<expr> AS <type>)
//! parser, analyzer, walker, format, GQLSTATUS, and runtime ISO §22 dispatch
//! matrix coverage. The CONFORMANCE-00 conformance-honesty bars (CAST records
//! GA05 "Cast specification"; GA05 claimed, GE08 not; corpus + CHANGELOG pins)
//! live in the sibling `cast_conformance.rs` so both files stay under the
//! 700-LOC cap.

use selene_core::{GraphId, Record, Value, intern};
use selene_gql::{
    AnalysisError, AnalyzedStatement, AnalyzedStatementKind, AnalyzedType, EmptyProcedureRegistry,
    GqlStatus, GqlType, PipelineStatement, ReturnItem, Session, Statement, StatementOutput,
    ValueExpr, analyze, ast::format::format_read_statement, parse,
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
fn cast_to_vector_type_is_syntax_error() {
    // `VECTOR` was removed-subsystem residue (post-#196 no-extensions pivot): it
    // is no longer a `type_name` alternative, so `CAST(x AS VECTOR)` fails to
    // match the grammar and reports a clean 42001 syntax error rather than a
    // deep not-implemented rejection.
    let err = parse("RETURN CAST(1 AS VECTOR) AS v").expect_err("CAST to VECTOR is rejected");
    assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR);
}

#[test]
fn vector_is_a_usable_bare_identifier() {
    // With the reserved `VECTOR` keyword removed, `vector` is an ordinary
    // identifier again and binds as a variable name.
    let statement = parse_or_panic("MATCH (vector) RETURN vector");
    assert!(matches!(statement, Statement::Query(_)));
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
fn cast_boolean_to_string_uppercase() {
    // ISO §20.8 GR4(j)(v)(1) / GR4v — boolean→string yields the UPPERCASE
    // literal "TRUE"/"FALSE" (810 strict-ISO fix; was lowercase).
    assert_eq!(
        as_string(execute_first_value("RETURN CAST(true AS STRING) AS v")),
        "TRUE"
    );
    assert_eq!(
        as_string(execute_first_value("RETURN CAST(false AS STRING) AS v")),
        "FALSE"
    );
}

#[test]
fn cast_string_to_boolean_case_insensitive() {
    // ISO §20.8 GR4(q) defers C→BO to the §21.2 boolean literal, which is
    // case-insensitive (810 strict-ISO fix; was strict-lowercase). Leading /
    // trailing whitespace is trimmed per GR4(g)(ii).
    for good in ["true", "True", "TRUE", "tRuE", "  true  "] {
        let source = format!("RETURN CAST('{good}' AS BOOLEAN) AS v");
        assert_eq!(
            execute_first_value(&source),
            Value::Bool(true),
            "input `{good}` must parse to TRUE"
        );
    }
    for good in ["false", "False", "FALSE", "fAlSe", " FALSE "] {
        let source = format!("RETURN CAST('{good}' AS BOOLEAN) AS v");
        assert_eq!(
            execute_first_value(&source),
            Value::Bool(false),
            "input `{good}` must parse to FALSE"
        );
    }
    // Non-boolean text still rejects with 22018.
    for bad in ["yes", "1", "t"] {
        let source = format!("RETURN CAST('{bad}' AS BOOLEAN) AS v");
        assert_eq!(
            execute_first_status(&source),
            "22018",
            "input `{bad}` must reject as 22018"
        );
    }
}

#[test]
fn cast_boolean_to_integer_returns_22g03() {
    // ISO §20.8 Table 4 marks BO→EN `N` — there is no boolean→numeric cast
    // (810 strict-ISO fix; the old 0/1 extension is removed). 22G03 datatype
    // mismatch.
    assert_eq!(
        execute_first_status("RETURN CAST(true AS INTEGER) AS v"),
        "22G03"
    );
    assert_eq!(
        execute_first_status("RETURN CAST(false AS INTEGER) AS v"),
        "22G03"
    );
}

#[test]
fn cast_boolean_to_float_returns_22g03() {
    // ISO §20.8 Table 4 marks BO→AN `N` (810 strict-ISO fix).
    assert_eq!(
        execute_first_status("RETURN CAST(true AS FLOAT) AS v"),
        "22G03"
    );
}

#[test]
fn cast_integer_to_boolean_returns_22g03() {
    // ISO §20.8 Table 4 marks EN→BO `N` — there is no numeric→boolean cast
    // (810 strict-ISO fix; the old 0/1 extension is removed). Every integer,
    // including 0/1, is now a 22G03 datatype mismatch.
    assert_eq!(
        execute_first_status("RETURN CAST(0 AS BOOLEAN) AS v"),
        "22G03"
    );
    assert_eq!(
        execute_first_status("RETURN CAST(1 AS BOOLEAN) AS v"),
        "22G03"
    );
    assert_eq!(
        execute_first_status("RETURN CAST(2 AS BOOLEAN) AS v"),
        "22G03"
    );
}

#[test]
fn cast_float_to_boolean_returns_22g03() {
    // ISO §20.8 Table 4 marks AN→BO `N` (810 strict-ISO fix).
    assert_eq!(
        execute_first_status("RETURN CAST(1.0 AS BOOLEAN) AS v"),
        "22G03"
    );
}

// ---------------------------------------------------------------------------
// 810 — DECIMAL CAST arms + numeric-family source widening (end-to-end).
// These thread the widened/decimal source variants through the full
// parse → analyze → plan → execute pipeline. Sources that have no GQL literal
// (Uint / Int128 / Uint128 / Float32 / Decimal) are bound as session
// parameters. The DECIMAL conversion helpers themselves are unit-tested in
// `runtime/evaluator/cast/decimal.rs`.
// ---------------------------------------------------------------------------

fn execute_with_param(source: &str, name: &str, value: Value) -> Value {
    let graph = SharedGraph::new(GraphId::new(13_530));
    let mut session = Session::new(&graph);
    session.bind_parameter(intern(name).expect("intern param"), value);
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    let StatementOutput::Rows(table) = output else {
        panic!("`{source}` produced no rows");
    };
    table
        .rows()
        .first()
        .and_then(|row| row.values().first())
        .cloned()
        .unwrap_or_else(|| panic!("`{source}` produced an empty row"))
}

fn execute_with_param_status(source: &str, name: &str, value: Value) -> String {
    let graph = SharedGraph::new(GraphId::new(13_531));
    let mut session = Session::new(&graph);
    session.bind_parameter(intern(name).expect("intern param"), value);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

#[test]
fn cast_string_to_decimal_and_back_round_trips() {
    // String → DECIMAL (GR4g(ii)) then DECIMAL → STRING (GR4j canonical).
    let value = execute_first_value("RETURN CAST(CAST('123.45' AS DECIMAL) AS STRING) AS v");
    assert_eq!(as_string(value), "123.45");
}

#[test]
fn cast_string_parse_fail_to_decimal_returns_22018() {
    assert_eq!(
        execute_first_status("RETURN CAST('not-a-number' AS DECIMAL) AS v"),
        "22018"
    );
}

#[test]
fn cast_decimal_source_to_integer_truncates() {
    // DECIMAL → INTEGER: truncate toward zero.
    let value = execute_first_value("RETURN CAST(CAST('3.7' AS DECIMAL) AS INTEGER) AS v");
    assert_eq!(value, Value::Int(3));
}

#[test]
fn cast_decimal_source_to_float() {
    let value = execute_first_value("RETURN CAST(CAST('2.5' AS DECIMAL) AS FLOAT) AS v");
    assert_eq!(value, Value::Float(2.5));
}

#[test]
fn cast_int_literal_to_decimal_then_string() {
    // Int → DECIMAL (GR4g) end-to-end via nested CAST.
    let value = execute_first_value("RETURN CAST(CAST(42 AS DECIMAL) AS STRING) AS v");
    assert_eq!(as_string(value), "42");
}

#[test]
fn cast_uint_param_to_integer_widens() {
    assert_eq!(
        execute_with_param("RETURN CAST($p AS INTEGER) AS v", "p", Value::Uint(7)),
        Value::Int(7)
    );
}

#[test]
fn cast_uint_param_above_i64_max_to_integer_returns_22003() {
    assert_eq!(
        execute_with_param_status(
            "RETURN CAST($p AS INTEGER) AS v",
            "p",
            Value::Uint(u64::MAX)
        ),
        "22003"
    );
}

#[test]
fn cast_int128_param_to_float_widens() {
    assert_eq!(
        execute_with_param("RETURN CAST($p AS FLOAT) AS v", "p", Value::Int128(-9)),
        Value::Float(-9.0)
    );
}

#[test]
fn cast_uint128_param_to_string_widens() {
    assert_eq!(
        as_string(execute_with_param(
            "RETURN CAST($p AS STRING) AS v",
            "p",
            Value::Uint128(123)
        )),
        "123"
    );
}

#[test]
fn cast_float32_param_to_integer_truncates() {
    assert_eq!(
        execute_with_param(
            "RETURN CAST($p AS INTEGER) AS v",
            "p",
            Value::Float32(3.7_f32)
        ),
        Value::Int(3)
    );
}

#[test]
fn cast_float_special_values_to_string_render_symbolically() {
    // `format_float` renders the non-finite FLOAT values symbolically per the
    // numeric→C (GR4j) shortest-conforming-literal path: NaN → "NaN",
    // ±Infinity → "Infinity"/"-Infinity". These have no GQL literal, so they
    // are threaded through a bound FLOAT parameter (the only reachable surface
    // for the `format_float` special-case branches).
    assert_eq!(
        as_string(execute_with_param(
            "RETURN CAST($p AS STRING) AS v",
            "p",
            Value::Float(f64::NAN)
        )),
        "NaN"
    );
    assert_eq!(
        as_string(execute_with_param(
            "RETURN CAST($p AS STRING) AS v",
            "p",
            Value::Float(f64::INFINITY)
        )),
        "Infinity"
    );
    assert_eq!(
        as_string(execute_with_param(
            "RETURN CAST($p AS STRING) AS v",
            "p",
            Value::Float(f64::NEG_INFINITY)
        )),
        "-Infinity"
    );
}

#[test]
fn cast_decimal_param_to_boolean_returns_22g03() {
    // ISO §20.8 Table 4 marks EN→BO `N`; DECIMAL is signed-exact (EN).
    assert_eq!(
        execute_with_param_status(
            "RETURN CAST($p AS BOOLEAN) AS v",
            "p",
            Value::Decimal("1".parse().unwrap())
        ),
        "22G03"
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
fn cast_record_to_scalar_returns_22g03() {
    // A record source to a non-record (scalar) target is an invalid type combination per
    // ISO §20.8 Table 4 (`N`) → 22G03 datatype mismatch (not a missing feature).
    let source = "RETURN CAST({a: 1, b: 2} AS STRING) AS v";
    assert_eq!(execute_first_status(source), "22G03");
}

#[test]
fn cast_record_to_closed_record_coerces_and_projects() {
    // ISO §20.8 GR4(e): per-field recursive cast (string '5' -> INT 5); SR12 subset
    // projection drops the undeclared extra source field `b`.
    let source = "RETURN CAST({a: '5', b: 2} AS RECORD{a :: INT}) AS v";
    let value = execute_first_value(source);
    assert_eq!(
        value,
        Value::Record(Box::new(Record::Open(
            [(intern("a").unwrap(), Value::Int(5))]
                .into_iter()
                .collect()
        )))
    );
}

#[test]
fn cast_record_missing_target_field_returns_22g0u() {
    // The target declares a field absent from the source → 22G0U record fields do not match.
    let source = "RETURN CAST({a: 1} AS RECORD{a :: INT, b :: STRING}) AS v";
    assert_eq!(execute_first_status(source), "22G0U");
}

#[test]
fn cast_non_record_to_record_returns_22g03() {
    let source = "RETURN CAST(5 AS RECORD{a :: INT}) AS v";
    assert_eq!(execute_first_status(source), "22G03");
}

#[test]
fn cast_record_to_open_record_is_identity() {
    let source = "RETURN CAST({a: 1, b: 'x'} AS RECORD) AS v";
    let value = execute_first_value(source);
    assert_eq!(
        value,
        Value::Record(Box::new(Record::Open(
            [
                (intern("a").unwrap(), Value::Int(1)),
                (intern("b").unwrap(), Value::String(intern("x").unwrap())),
            ]
            .into_iter()
            .collect()
        )))
    );
}

#[test]
fn cast_null_to_record_is_null() {
    let source = "RETURN CAST(NULL AS RECORD{a :: INT}) AS v";
    assert_eq!(execute_first_value(source), Value::Null);
}

#[test]
fn cast_record_inner_field_cast_failure_propagates() {
    // ISO §20.8 GR4(e)(i): each field is cast recursively, so a failing inner cast
    // ('abc' -> INT) must surface its own status (22018 invalid character value for cast),
    // not a swallowed or relabelled error.
    let source = "RETURN CAST({a: 'abc'} AS RECORD{a :: INT}) AS v";
    assert_eq!(execute_first_status(source), "22018");
}

#[test]
fn cast_record_to_nested_record_coerces_recursively() {
    // The recursive-descent path (cast_to_record re-enters under stacker::maybe_grow): a
    // nested closed-record target coerces the inner field ('5' -> INT 5).
    let source = "RETURN CAST({a: {b: '5'}} AS RECORD{a :: RECORD{b :: INT}}) AS v";
    let value = execute_first_value(source);
    assert_eq!(
        value,
        Value::Record(Box::new(Record::Open(
            [(
                intern("a").unwrap(),
                Value::Record(Box::new(Record::Open(
                    [(intern("b").unwrap(), Value::Int(5))]
                        .into_iter()
                        .collect()
                )))
            )]
            .into_iter()
            .collect()
        )))
    );
}

#[test]
fn cast_anything_to_null_returns_42n01() {
    // §A.3 — CAST to GqlType::Null is rejected as 42N01 (cannot cast TO
    // null). The grammar requires SQL-style explicit NULL as a type token;
    // selene-db's type grammar accepts `NULL` per `ast/types.rs:84`.
    let source = "RETURN CAST(1 AS NULL) AS v";
    assert_eq!(execute_first_status(source), "42N01");
}
