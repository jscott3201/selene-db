//! End-to-end coverage for the native [`BuiltinProcedureRegistry`].
//!
//! These tests drive `CALL algo.*` through the concrete native registry (no
//! procedure-pack machinery), exercising plan-time lookup, tier-checked
//! dispatch, the per-`GraphId` projection lifecycle, and a real algorithm run.
//! They are the STEP-2 parity guard: every `algo.*` name resolves and executes
//! identically to the pack era, with the registry swapped behind the
//! `ProcedureRegistry` trait.

use selene_core::{GraphId, IStr, Value, intern};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ProcedureRegistry, Session, StatementOutput,
};
use selene_graph::SharedGraph;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("written statement returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn execute_rows(
    session: &mut Session<'_>,
    source: &str,
    registry: &dyn ProcedureRegistry,
) -> BindingTable {
    rows(
        session
            .execute_source(source, registry)
            .expect("statement executes"),
    )
}

fn float_column(table: &BindingTable, name: &str) -> Vec<f64> {
    let index = table
        .column_index(istr(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::Float(value)) => *value,
            other => panic!("expected float in {name}, got {other:?}"),
        })
        .collect()
}

fn string_column(table: &BindingTable, name: &str) -> Vec<String> {
    let index = table
        .column_index(istr(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::String(value)) => value.as_str().to_owned(),
            other => panic!("expected string in {name}, got {other:?}"),
        })
        .collect()
}

/// Build a small directed triangle graph: (a)->(b)->(c)->(a).
fn seed_triangle(session: &mut Session<'_>, registry: &dyn ProcedureRegistry) {
    session
        .execute_source(
            "INSERT (a:N)-[:E]->(b:N), (b)-[:E]->(c:N), (c)-[:E]->(a)",
            registry,
        )
        .expect("seed graph inserts");
}

#[test]
fn show_procedures_lists_all_nineteen_algo_procedures() {
    let graph = graph(220_001);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let table = execute_rows(&mut session, "SHOW PROCEDURES", &registry);
    let names = string_column(&table, "name");

    // The registry also carries the 20 `selene.*` platform built-ins, so SHOW
    // PROCEDURES lists 39; all 19 algo names must still be present.
    assert_eq!(
        table.row_count(),
        39,
        "expected 19 algo procedures + 20 platform built-ins"
    );
    for expected in [
        "algo.projection_build",
        "algo.projection_get",
        "algo.projection_drop",
        "algo.projection_list",
        "algo.pagerank",
        "algo.betweenness",
        "algo.label_propagation",
        "algo.louvain",
        "algo.triangle_count",
        "algo.wcc",
        "algo.scc",
        "algo.wcc_count",
        "algo.scc_count",
        "algo.topological_sort",
        "algo.articulation_points",
        "algo.bridges",
        "algo.dijkstra",
        "algo.sssp",
        "algo.apsp",
    ] {
        assert!(
            names.contains(&expected.to_owned()),
            "SHOW PROCEDURES must list {expected}"
        );
    }
}

#[test]
fn pagerank_runs_end_to_end_over_a_built_projection() {
    let graph = graph(220_002);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    seed_triangle(&mut session, &registry);

    session
        .execute_source(
            "CALL algo.projection_build('p', NULL, NULL, NULL)",
            &registry,
        )
        .expect("projection_build executes");

    let table = execute_rows(
        &mut session,
        "CALL algo.pagerank('p', NULL, NULL, NULL, NULL) YIELD node_id, score",
        &registry,
    );

    // Three nodes in the triangle, each with a finite positive score summing to
    // ~1.0 (PageRank teleport-normalized over a symmetric cycle).
    let scores = float_column(&table, "score");
    assert_eq!(scores.len(), 3, "one score row per node");
    assert!(
        scores.iter().all(|s| s.is_finite() && *s > 0.0),
        "scores must be finite and positive: {scores:?}"
    );
    let total: f64 = scores.iter().sum();
    assert!(
        (total - 1.0).abs() < 1e-6,
        "pagerank scores should sum to ~1.0, got {total}"
    );
}

#[test]
fn wcc_count_yields_single_component_for_connected_graph() {
    let graph = graph(220_003);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    seed_triangle(&mut session, &registry);

    session
        .execute_source(
            "CALL algo.projection_build('p', NULL, NULL, NULL)",
            &registry,
        )
        .expect("projection_build executes");

    let table = execute_rows(
        &mut session,
        "CALL algo.wcc_count('p') YIELD count",
        &registry,
    );

    let index = table.column_index(istr("count")).expect("count column");
    let count = match table.rows()[0].values().get(index) {
        Some(Value::Uint(value)) => *value,
        other => panic!("expected uint count, got {other:?}"),
    };
    assert_eq!(
        count, 1,
        "a directed triangle is one weakly connected component"
    );
}

#[test]
fn projection_list_reports_built_projection() {
    let graph = graph(220_004);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    seed_triangle(&mut session, &registry);

    session
        .execute_source(
            "CALL algo.projection_build('proj_a', NULL, NULL, NULL)",
            &registry,
        )
        .expect("projection_build executes");

    let table = execute_rows(
        &mut session,
        "CALL algo.projection_list() YIELD name, node_count, edge_count",
        &registry,
    );
    let names = string_column(&table, "name");
    assert_eq!(names, vec!["proj_a".to_owned()]);
}

#[test]
fn pagerank_on_missing_projection_is_an_invalid_argument_error() {
    let graph = graph(220_005);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    seed_triangle(&mut session, &registry);

    let err = session
        .execute_source(
            "CALL algo.pagerank('does_not_exist', NULL, NULL, NULL, NULL) YIELD node_id, score",
            &registry,
        )
        .expect_err("missing projection must error");
    let rendered = format!("{err:?}");
    assert!(
        rendered.to_lowercase().contains("projection")
            || rendered.to_lowercase().contains("does_not_exist"),
        "error should mention the missing projection, got: {rendered}"
    );
}

#[test]
fn unknown_algo_procedure_is_rejected_at_plan_time() {
    let graph = graph(220_006);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let err = session
        .execute_source("CALL algo.does_not_exist('p')", &registry)
        .expect_err("unknown procedure must be rejected");
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("does_not_exist") || rendered.to_lowercase().contains("procedure"),
        "unknown procedure error should name the procedure, got: {rendered}"
    );
}

#[test]
fn forget_graph_clears_projection_state() {
    let graph = graph(220_007);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    seed_triangle(&mut session, &registry);

    session
        .execute_source(
            "CALL algo.projection_build('p', NULL, NULL, NULL)",
            &registry,
        )
        .expect("projection_build executes");

    // The catalog for this graph now exists; forgetting it returns true, and a
    // subsequent forget returns false (state was reclaimed).
    assert!(registry.forget_graph(GraphId::new(220_007)));
    assert!(!registry.forget_graph(GraphId::new(220_007)));

    // After reclamation, the projection is gone: list reports nothing.
    let table = execute_rows(
        &mut session,
        "CALL algo.projection_list() YIELD name",
        &registry,
    );
    assert_eq!(table.row_count(), 0, "projection state cleared on forget");
}
