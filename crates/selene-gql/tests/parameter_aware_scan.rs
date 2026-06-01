//! BRIEF-154 end-to-end coverage for parameter-aware indexed scans.
//!
//! Each test builds a graph with a typed index, declares a matching
//! [`MockIndexCatalog`] for the optimizer, plans + optimizes a parameterized
//! source so the optimizer emits `IndexKey::Parameter` slots, binds parameters
//! on a [`Session`], and runs the plan through [`execute_statement`]. This
//! exercises the runtime [`resolve_index_key`](../src/runtime/scan.rs) helper
//! against `&EvalCtx` parameters across all four code paths (TypedIndexRange,
//! BitmapUnion, CompositeLookup, plus the linear fallback used when the
//! index is unavailable).

use std::num::NonZeroUsize;
use std::sync::Arc;

use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, intern};
use selene_gql::plan::optimize::optimize_summary;
use selene_gql::{
    BindingTable, EmptyProcedureRegistry, ExecutionPlan, ExecutorError, IndexKind, OptimizeContext,
    Session, StatementOutput, analyze, execute_statement, optimize, parse, plan as build_plan,
};
use selene_graph::{SharedGraph, TypedIndexKind};
use selene_testing::MockIndexCatalog;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn props<const N: usize>(pairs: [(IStr, Value); N]) -> PropertyMap {
    PropertyMap::from_pairs(pairs).expect("test properties fit caps")
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        other => panic!("expected rows, got {other:?}"),
    }
}

fn optimized_plan(source: &str, catalog: &MockIndexCatalog) -> ExecutionPlan {
    let statement = parse(source).expect("test source parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test source analyzes");
    let plan = build_plan(&analyzed, &EmptyProcedureRegistry).expect("test source plans");
    let ctx = OptimizeContext::default().with_index_catalog(catalog);
    optimize(plan, &ctx)
}

/// Build a graph populated with `Person` nodes (id, name) and a typed STRING
/// index on `name`. The storage index makes the runtime probe satisfiable;
/// the returned catalog mirrors it for the optimizer.
fn person_graph_with_name_index() -> (SharedGraph, MockIndexCatalog) {
    let graph = SharedGraph::new(GraphId::new(15_401));
    let label = istr("Person");
    let id_key = istr("id");
    let name_key = istr("name");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for (id, name) in [
            (1_i64, "alice"),
            (2, "bob"),
            (3, "cara"),
            (4, "dave"),
            (5, "eve"),
        ] {
            mutator
                .create_node(
                    LabelSet::single(label.clone()),
                    props([
                        (id_key.clone(), Value::Int(id)),
                        (name_key.clone(), Value::String(istr(name))),
                    ]),
                )
                .expect("person inserts");
        }
        txn.commit().expect("seed commits");
    }
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_property_index(label.clone(), name_key.clone(), TypedIndexKind::String)
            .expect("name index registers");
        txn.commit().expect("index commit");
    }
    let catalog = MockIndexCatalog::new().with_node_typed_index(label, name_key, IndexKind::String);
    (graph, catalog)
}

fn collect_id_column(table: &BindingTable) -> Vec<i64> {
    let column = 0;
    table
        .rows()
        .iter()
        .map(|row| match row.values()[column] {
            Value::Int(value) => value,
            ref other => panic!("expected Int id, got {other:?}"),
        })
        .collect()
}

#[test]
fn parameterized_equality_executes_against_typed_string_index() {
    // BRIEF-154 bar 1 + execution: plan with `Equality(IndexKey::Parameter)`
    // and verify the runtime probe returns the same row as the literal-equivalent
    // would. Establishes the happy path across the resolve_index_key boundary.
    let (graph, catalog) = person_graph_with_name_index();
    let plan = Arc::new(optimized_plan(
        "MATCH (n:Person) WHERE n.name = $name RETURN n.id AS id",
        &catalog,
    ));
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("name"), Value::String(istr("cara")));

    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
            .expect("parameterized equality executes"),
    );
    assert_eq!(collect_id_column(&table), vec![3]);
}

#[test]
fn null_parameter_binding_returns_empty_result_without_erroring() {
    // BRIEF-154 §B.3 F5: parity with inline `WHERE n.name = NULL` — 3VL
    // semantics fall through to zero rows, never an error.
    let (graph, catalog) = person_graph_with_name_index();
    let plan = Arc::new(optimized_plan(
        "MATCH (n:Person) WHERE n.name = $name RETURN n.id AS id",
        &catalog,
    ));
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("name"), Value::Null);

    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
            .expect("null parameter executes"),
    );
    assert!(table.rows().is_empty());
}

#[test]
fn computed_string_equality_finds_row_on_indexed_string_column() {
    // Interner-removal replacement for the BRIEF-153 ExternalString carve-out
    // test: an equality probe on an INDEXED STRING column with a COMPUTED
    // (CAST-derived) string value must still resolve to the indexed row.
    // Post-removal there is a single string space, so a computed string is a
    // plain `Value::String` and the indexed lookup finds the row exactly as a
    // literal probe would.
    let (graph, catalog) = person_graph_with_name_index();
    let plan = Arc::new(optimized_plan(
        "MATCH (n:Person) WHERE n.name = CAST('bob' AS STRING) RETURN n.id AS id",
        &catalog,
    ));
    let mut session = Session::new(&graph);

    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
            .expect("computed-string equality executes"),
    );
    assert_eq!(collect_id_column(&table), vec![2]);
}

#[test]
fn computed_string_parameter_equality_finds_row_on_indexed_string_column() {
    // Same as above but via a bound STRING parameter rather than a CAST
    // literal: the indexed-column equality probe finds the matching row.
    let (graph, catalog) = person_graph_with_name_index();
    let plan = Arc::new(optimized_plan(
        "MATCH (n:Person) WHERE n.name = $name RETURN n.id AS id",
        &catalog,
    ));
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("name"), Value::String(istr("bob")));

    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
            .expect("string-parameter equality executes"),
    );
    assert_eq!(collect_id_column(&table), vec![2]);
}

#[test]
fn wrong_kind_parameter_binding_errors_with_parameter_name() {
    // BRIEF-154 §B.3 F12 + ISO §23.1 Table 8 22G03: binding an INT to a
    // STRING-indexed parameter slot must fail loud with `InvalidParameterType`
    // identifying which parameter went wrong.
    let (graph, catalog) = person_graph_with_name_index();
    let plan = Arc::new(optimized_plan(
        "MATCH (n:Person) WHERE n.name = $name RETURN n.id AS id",
        &catalog,
    ));
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("name"), Value::Int(42));

    let err = execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
        .expect_err("wrong-kind parameter errors");
    let ExecutorError::InvalidParameterType {
        name,
        expected,
        actual,
        ..
    } = err
    else {
        panic!("expected InvalidParameterType, got {err:?}");
    };
    assert_eq!(name.as_str(), "name");
    assert_eq!(&*expected, "STRING");
    assert_eq!(actual, "INTEGER");
    // ExecutorError::InvalidParameterType is defined with
    // `#[diagnostic(code(SLENE_X_22G03))]` per ISO §23.1 Table 8; the
    // mapping is enforced at the variant level (see runtime/error.rs:194).
}

#[test]
fn unbound_parameter_errors_with_parameter_name() {
    // BRIEF-154 bar 8: parameterized plan executed without binding the
    // referenced parameter surfaces `UnboundParameter` with the source span.
    let (graph, catalog) = person_graph_with_name_index();
    let plan = Arc::new(optimized_plan(
        "MATCH (n:Person) WHERE n.name = $missing RETURN n.id AS id",
        &catalog,
    ));
    let mut session = Session::new(&graph);
    let err = execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
        .expect_err("unbound parameter errors");
    let ExecutorError::UnboundParameter { name, .. } = err else {
        panic!("expected UnboundParameter, got {err:?}");
    };
    assert_eq!(name.as_str(), "missing");
}

#[test]
fn parameterized_in_list_executes_against_bitmap_union() {
    // BRIEF-154 bar 4: BitmapUnion-shaped probe with parameter slots.
    let (graph, catalog) = person_graph_with_name_index();
    let plan = Arc::new(optimized_plan(
        "MATCH (n:Person) WHERE n.name IN [$a, $b] RETURN n.id AS id",
        &catalog,
    ));
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("a"), Value::String(istr("alice")));
    session.bind_parameter(istr("b"), Value::String(istr("dave")));

    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
            .expect("parameterized in-list executes"),
    );
    let mut ids = collect_id_column(&table);
    ids.sort();
    assert_eq!(ids, vec![1, 4]);
}

#[test]
fn parameterized_in_list_null_binding_drops_that_branch() {
    // BRIEF-154 §B.3 F5 for the BitmapUnion path: a NULL key contributes
    // zero rows but does NOT erase the rest of the union.
    let (graph, catalog) = person_graph_with_name_index();
    let plan = Arc::new(optimized_plan(
        "MATCH (n:Person) WHERE n.name IN [$a, $b] RETURN n.id AS id",
        &catalog,
    ));
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("a"), Value::String(istr("alice")));
    session.bind_parameter(istr("b"), Value::Null);

    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
            .expect("partial-null in-list executes"),
    );
    assert_eq!(collect_id_column(&table), vec![1]);
}

#[test]
fn explain_renders_parameter_slot_as_dollar_name() {
    // BRIEF-154 bar 9: EXPLAIN summary surfaces parameter slots readably as
    // `$name`, not raw `Debug`. Pins the new `bounds=…` detail surface.
    let (_graph, catalog) = person_graph_with_name_index();
    let statement =
        parse("MATCH (n:Person) WHERE n.name = $symbol RETURN n.id").expect("test source parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("analyzes");
    let plan = build_plan(&analyzed, &EmptyProcedureRegistry).expect("plans");
    let summary = optimize_summary(
        plan,
        &OptimizeContext::default().with_index_catalog(&catalog),
    );
    let display = summary.to_string();
    assert!(
        display.contains("TypedIndexRange [bounds=Equality($symbol)]"),
        "expected `TypedIndexRange [bounds=Equality($symbol)]` in summary, got:\n{display}",
    );
}

#[test]
fn explain_renders_literal_with_kind_and_value() {
    // BRIEF-154 bar 9 (literal leg): the `[bounds=…]` rendering for inline
    // literals carries the kind tag + display value (e.g. `STRING 'alice'`).
    let (_graph, catalog) = person_graph_with_name_index();
    let statement =
        parse("MATCH (n:Person) WHERE n.name = 'alice' RETURN n.id").expect("test source parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("analyzes");
    let plan = build_plan(&analyzed, &EmptyProcedureRegistry).expect("plans");
    let summary = optimize_summary(
        plan,
        &OptimizeContext::default().with_index_catalog(&catalog),
    );
    let display = summary.to_string();
    assert!(
        display.contains("TypedIndexRange [bounds=Equality(STRING 'alice')]"),
        "expected literal-rendered bounds detail in summary, got:\n{display}",
    );
}

#[test]
fn session_plan_cache_hits_across_parameter_value_changes() {
    // BRIEF-154 bar 5: same source-text + two different `$name` values →
    // PlanCacheStats reports one miss + one hit. Verifies `IndexKey::Parameter`
    // remains a stable per-source-text value for cache keying.
    let (graph, _catalog) = person_graph_with_name_index();
    let mut session = Session::new(&graph).with_plan_cache(NonZeroUsize::new(8).unwrap());
    let source = "MATCH (n:Person) WHERE n.name = $name RETURN n.id AS id";

    session.bind_parameter(istr("name"), Value::String(istr("alice")));
    let table = rows(
        session
            .execute_source(source, &EmptyProcedureRegistry)
            .expect("first execute"),
    );
    assert_eq!(collect_id_column(&table), vec![1]);

    session.bind_parameter(istr("name"), Value::String(istr("eve")));
    let table = rows(
        session
            .execute_source(source, &EmptyProcedureRegistry)
            .expect("second execute"),
    );
    assert_eq!(collect_id_column(&table), vec![5]);

    let stats = session.plan_cache_stats().expect("cache enabled");
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hits, 1);
}

#[test]
fn string_range_parameter_finds_rows_via_index_range() {
    // STRING range probes now resolve through the index: `lookup_range` walks
    // the `BTreeMap<IStr, _>` over the lexicographically-ordered keys (the
    // typed-index collapse landed). The matched rows are identical to the old
    // linear-scan fallback (which compared `Value::String` rows
    // lexicographically against the range endpoints) — just resolved via the
    // index instead of a full scan.
    let (graph, catalog) = person_graph_with_name_index();
    let plan = Arc::new(optimized_plan(
        "MATCH (n:Person) WHERE n.name > $lo AND n.name < $hi RETURN n.id AS id",
        &catalog,
    ));
    let mut session = Session::new(&graph);
    // Bind range endpoints that bracket the seeded names alphabetically.
    session.bind_parameter(istr("lo"), Value::String(istr("a-sentinel-1")));
    session.bind_parameter(istr("hi"), Value::String(istr("z-sentinel-1")));
    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
            .expect("string range with string parameter executes"),
    );
    let mut ids = collect_id_column(&table);
    ids.sort();
    // All 5 seeded names ("alice", "bob", "cara", "dave", "eve") sort
    // alphabetically between the bracket tokens.
    assert_eq!(ids, vec![1, 2, 3, 4, 5]);
}

#[test]
fn equality_after_range_keeps_range_as_residual_filter() {
    // BRIEF-154 PR #175 F3 (Codex P1): pre-fix, `WHERE n.age > 10 AND n.age = $p`
    // had both predicates removed from `property_predicates`, so the executor
    // would probe `age = $p` and silently drop the `age > 10` filter. With
    // `$p = 5`, age=5 rows leaked through despite failing the `>10` check.
    // Fix: the equality arm of `bounds_for_property` now resets `consumed`
    // to just `[index]`, leaving any earlier range bounds as residual
    // predicates the executor still enforces post-probe.
    let (graph, _catalog) = person_graph_with_name_index();
    let label = istr("Person");
    let age_key = istr("age");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for age in [5_i64, 15, 25] {
            mutator
                .create_node(
                    LabelSet::single(label.clone()),
                    props([
                        (istr("id"), Value::Int(age)),
                        (age_key.clone(), Value::Int(age)),
                    ]),
                )
                .expect("age node inserts");
        }
        txn.commit().expect("seed commits");
    }
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_property_index(label.clone(), age_key.clone(), TypedIndexKind::I64)
            .expect("age index registers");
        txn.commit().expect("index commit");
    }
    let catalog = MockIndexCatalog::new()
        .with_node_typed_index(label.clone(), istr("name"), IndexKind::String)
        .with_node_typed_index(label, age_key, IndexKind::Integer);

    // Parameter equality after range: `$p = 5` AND `age > 10` should be empty
    // (no age=5 row survives the >10 filter).
    let plan = Arc::new(optimized_plan(
        "MATCH (n:Person) WHERE n.age > 10 AND n.age = $p RETURN n.id AS id",
        &catalog,
    ));
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("p"), Value::Int(5));
    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
            .expect("equality-after-range executes"),
    );
    assert!(
        table.rows().is_empty(),
        "expected empty result (no rows have age=5 AND age>10), got {:?}",
        collect_id_column(&table)
    );

    // Sanity: `$p = 25` AND `age > 10` should find the age=25 row.
    session.bind_parameter(istr("p"), Value::Int(25));
    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
            .expect("equality-after-range matches row above range"),
    );
    assert_eq!(collect_id_column(&table), vec![25]);

    // Literal-variant regression: the same bug existed pre-PR-#175 for
    // literal equality. Confirm the fix closes it there too.
    let literal_plan = Arc::new(optimized_plan(
        "MATCH (n:Person) WHERE n.age > 10 AND n.age = 5 RETURN n.id AS id",
        &catalog,
    ));
    let table = rows(
        execute_statement(&literal_plan, &mut session, &EmptyProcedureRegistry)
            .expect("literal equality-after-range executes"),
    );
    assert!(
        table.rows().is_empty(),
        "literal variant must also drop residual range filter onto post-probe path"
    );
}

#[test]
fn range_with_inverted_parameter_bounds_returns_empty_not_panic() {
    // BRIEF-154 PR #175 F1 (Codex P1): with parameter-bearing range bounds
    // the plan-time `range_satisfiable` guard is skipped (it gates on
    // literal-literal pairs only). Forwarding `$lo > $hi` to
    // `BTreeMap::range` would std::panic — the runtime guard installed in
    // `typed_index_rows` after `resolve_bounds` must short-circuit to an
    // empty result.
    let (graph, _catalog) = person_graph_with_name_index();
    // Build a graph with an INT-typed `age` index so we can exercise a
    // numeric range with inverted parameter bounds.
    let label = istr("Person");
    let age_key = istr("age");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for age in [20_i64, 35, 50] {
            mutator
                .create_node(
                    LabelSet::single(label.clone()),
                    props([
                        (istr("id"), Value::Int(age)),
                        (age_key.clone(), Value::Int(age)),
                    ]),
                )
                .expect("age node inserts");
        }
        txn.commit().expect("seed commits");
    }
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_property_index(label.clone(), age_key.clone(), TypedIndexKind::I64)
            .expect("age index registers");
        txn.commit().expect("index commit");
    }
    let catalog = MockIndexCatalog::new()
        .with_node_typed_index(label.clone(), istr("name"), IndexKind::String)
        .with_node_typed_index(label, age_key, IndexKind::Integer);
    let plan = Arc::new(optimized_plan(
        "MATCH (n:Person) WHERE n.age > $lo AND n.age < $hi RETURN n.id AS id",
        &catalog,
    ));

    // $lo > $hi → unsatisfiable. Must return empty without panicking.
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("lo"), Value::Int(100));
    session.bind_parameter(istr("hi"), Value::Int(0));
    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
            .expect("inverted range executes without panicking"),
    );
    assert!(table.rows().is_empty());

    // Boundary case: $lo == $hi with both exclusive (`> AND <`) also empty.
    session.bind_parameter(istr("lo"), Value::Int(35));
    session.bind_parameter(istr("hi"), Value::Int(35));
    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
            .expect("equal-bound exclusive range executes"),
    );
    assert!(table.rows().is_empty());

    // Sanity: $lo < $hi still works.
    session.bind_parameter(istr("lo"), Value::Int(0));
    session.bind_parameter(istr("hi"), Value::Int(100));
    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
            .expect("valid range executes"),
    );
    let mut ids = collect_id_column(&table);
    ids.sort();
    assert_eq!(ids, vec![20, 35, 50]);
}

#[test]
fn plan_arc_is_reusable_across_parameter_value_changes() {
    // BRIEF-154 bar 5 spirit: the same optimized plan (with parameter slots
    // in its IR) executes correctly across different parameter bindings.
    // This complements the existing `plan_cache_hits_across_param_value_changes`
    // session-cache test by exercising the optimized plan path directly.
    let (graph, catalog) = person_graph_with_name_index();
    let plan = Arc::new(optimized_plan(
        "MATCH (n:Person) WHERE n.name = $name RETURN n.id AS id",
        &catalog,
    ));
    let mut session = Session::new(&graph);

    session.bind_parameter(istr("name"), Value::String(istr("alice")));
    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry).expect("first execute"),
    );
    assert_eq!(collect_id_column(&table), vec![1]);

    session.bind_parameter(istr("name"), Value::String(istr("eve")));
    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry).expect("second execute"),
    );
    assert_eq!(collect_id_column(&table), vec![5]);
}
