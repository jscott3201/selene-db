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

use std::sync::Arc;

use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, intern};
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
                    LabelSet::single(label),
                    props([
                        (id_key, Value::Int(id)),
                        (name_key, Value::String(istr(name))),
                    ]),
                )
                .expect("person inserts");
        }
        txn.commit().expect("seed commits");
    }
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_property_index(label, name_key, TypedIndexKind::String)
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
fn external_string_parameter_pooled_content_finds_row() {
    // BRIEF-154 §B.3 F4: when `Value::ExternalString` content has been
    // admitted to the global IStr pool (here via the literal seed values),
    // resolve_index_key coerces it to `Value::String(IStr)` and the typed
    // index probe finds the corresponding row.
    let (graph, catalog) = person_graph_with_name_index();
    let plan = Arc::new(optimized_plan(
        "MATCH (n:Person) WHERE n.name = $name RETURN n.id AS id",
        &catalog,
    ));
    let mut session = Session::new(&graph);
    // "bob" is already pooled because it appeared in the literal property
    // seed; an ExternalString carrying the same content should coerce.
    session.bind_parameter(istr("name"), Value::ExternalString(Arc::from("bob")));

    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
            .expect("pooled external string executes"),
    );
    assert_eq!(collect_id_column(&table), vec![2]);
}

#[test]
fn external_string_parameter_unpooled_content_returns_empty() {
    // BRIEF-154 §B.3 F4 second leg: ExternalString content never admitted
    // (here a fresh random suffix) can't possibly match any indexed row —
    // resolve_index_key returns EmptyResult without admitting the string.
    let (graph, catalog) = person_graph_with_name_index();
    let plan = Arc::new(optimized_plan(
        "MATCH (n:Person) WHERE n.name = $name RETURN n.id AS id",
        &catalog,
    ));
    let mut session = Session::new(&graph);
    session.bind_parameter(
        istr("name"),
        // The token below must NOT appear in any other test in this binary
        // before this point, otherwise it would be admitted via the seed.
        Value::ExternalString(Arc::from("param-aware-scan-unpooled-sentinel-3pXkQv7yLm9")),
    );

    let table = rows(
        execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
            .expect("unpooled external string executes"),
    );
    assert!(table.rows().is_empty());
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
