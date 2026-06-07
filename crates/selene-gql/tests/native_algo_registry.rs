//! End-to-end coverage for the native [`BuiltinProcedureRegistry`].
//!
//! These tests drive `CALL algo.*` through the concrete native registry (no
//! procedure-pack machinery), exercising plan-time lookup, tier-checked
//! dispatch, the per-`GraphId` projection lifecycle, and a real algorithm run.
//! They are the STEP-2 parity guard: every `algo.*` name resolves and executes
//! identically to the pack era, with the registry swapped behind the
//! `ProcedureRegistry` trait.

use selene_core::{DbString, GraphId, LabelSet, NodeId, PropertyMap, Record, Value};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ProcedureRegistry, Session, StatementOutput,
};
use selene_graph::SharedGraph;
use smallvec::smallvec;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn seed_isolated_nodes(graph: &SharedGraph, count: usize) -> Vec<NodeId> {
    let mut txn = graph.begin_write();
    let label = db_string("N");
    let mut nodes = Vec::with_capacity(count);
    for _ in 0..count {
        nodes.push(
            txn.mutator()
                .create_node(LabelSet::single(label.clone()), PropertyMap::new())
                .expect("test node creates"),
        );
    }
    txn.commit().expect("test graph commit");
    nodes
}

fn seed_sink_personalization_graph(graph: &SharedGraph) -> Vec<NodeId> {
    let mut txn = graph.begin_write();
    let label = db_string("N");
    let rel = db_string("R");
    let mut nodes = Vec::with_capacity(3);
    for _ in 0..3 {
        nodes.push(
            txn.mutator()
                .create_node(LabelSet::single(label.clone()), PropertyMap::new())
                .expect("test node creates"),
        );
    }
    txn.mutator()
        .create_edge(rel.clone(), nodes[0], nodes[1], PropertyMap::new())
        .expect("fact edge creates");
    txn.mutator()
        .create_edge(rel, nodes[2], nodes[1], PropertyMap::new())
        .expect("episode edge creates");
    txn.commit().expect("test graph commit");
    nodes
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
        .column_index(db_string(name))
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

fn node_column(table: &BindingTable, name: &str) -> Vec<NodeId> {
    let index = table
        .column_index(db_string(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::NodeRef(value)) => *value,
            other => panic!("expected node ref in {name}, got {other:?}"),
        })
        .collect()
}

fn string_column(table: &BindingTable, name: &str) -> Vec<String> {
    let index = table
        .column_index(db_string(name))
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

fn personalization_seed(node: NodeId, weight: f64) -> Value {
    Value::Record(Box::new(Record::Open(smallvec![
        (db_string("node_id"), Value::NodeRef(node)),
        (db_string("weight"), Value::Float(weight)),
    ])))
}

#[test]
fn show_procedures_lists_all_nineteen_algo_procedures() {
    let graph = graph(220_001);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let table = execute_rows(&mut session, "SHOW PROCEDURES", &registry);
    let names = string_column(&table, "name");

    // The registry also carries the 45 `selene.*` platform built-ins, so SHOW
    // PROCEDURES lists 64; all 19 algo names must still be present.
    assert_eq!(
        table.row_count(),
        64,
        "expected 19 algo procedures + 45 platform built-ins"
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
fn pagerank_accepts_personalization_parameter() {
    let graph = graph(220_008);
    let nodes = seed_isolated_nodes(&graph, 3);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    session
        .execute_source(
            "CALL algo.projection_build('p', NULL, NULL, NULL)",
            &registry,
        )
        .expect("projection_build executes");
    session.bind_parameter(
        db_string("seeds"),
        Value::List(vec![personalization_seed(nodes[2], 1.0)]),
    );

    let table = execute_rows(
        &mut session,
        "CALL algo.pagerank('p', 0.85, 10, 0.0, NULL, 'NATURAL', $seeds) YIELD node_id, score",
        &registry,
    );

    let result_nodes = node_column(&table, "node_id");
    let scores = float_column(&table, "score");
    assert_eq!(result_nodes[0], nodes[2]);
    assert!((scores[0] - 1.0).abs() < 1e-12);
    assert!((scores[1] - 0.0).abs() < 1e-12);
    assert!((scores[2] - 0.0).abs() < 1e-12);
}

#[test]
fn pagerank_undirected_orientation_spreads_personalized_sink_seed() {
    let graph = graph(220_010);
    let nodes = seed_sink_personalization_graph(&graph);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    session
        .execute_source(
            "CALL algo.projection_build('p', NULL, NULL, NULL)",
            &registry,
        )
        .expect("projection_build executes");
    session.bind_parameter(
        db_string("seeds"),
        Value::List(vec![personalization_seed(nodes[1], 1.0)]),
    );

    let natural = execute_rows(
        &mut session,
        "CALL algo.pagerank('p', 0.85, 1, 0.0, NULL, 'NATURAL', $seeds) YIELD node_id, score",
        &registry,
    );
    let undirected = execute_rows(
        &mut session,
        "CALL algo.pagerank('p', 0.85, 1, 0.0, NULL, 'UNDIRECTED', $seeds) YIELD node_id, score",
        &registry,
    );
    let score_for = |table: &BindingTable, node| {
        node_column(table, "node_id")
            .into_iter()
            .zip(float_column(table, "score"))
            .find(|(candidate, _)| *candidate == node)
            .expect("node has score")
            .1
    };

    assert!((score_for(&natural, nodes[1]) - 1.0).abs() < 1e-12);
    assert!((score_for(&natural, nodes[0]) - 0.0).abs() < 1e-12);
    assert!((score_for(&natural, nodes[2]) - 0.0).abs() < 1e-12);
    assert!((score_for(&undirected, nodes[1]) - 0.15).abs() < 1e-12);
    assert!((score_for(&undirected, nodes[0]) - 0.425).abs() < 1e-12);
    assert!((score_for(&undirected, nodes[2]) - 0.425).abs() < 1e-12);
}

#[test]
fn pagerank_personalization_rejects_seed_outside_projection() {
    let graph = graph(220_009);
    seed_isolated_nodes(&graph, 2);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    session
        .execute_source(
            "CALL algo.projection_build('p', NULL, NULL, NULL)",
            &registry,
        )
        .expect("projection_build executes");
    session.bind_parameter(
        db_string("seeds"),
        Value::List(vec![personalization_seed(NodeId::new(999), 1.0)]),
    );

    let err = session
        .execute_source(
            "CALL algo.pagerank('p', 0.85, 10, 0.0, NULL, 'NATURAL', $seeds) YIELD node_id, score",
            &registry,
        )
        .expect_err("out-of-projection seed rejected");
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("not in projection") && rendered.contains("999"),
        "error should mention out-of-projection seed, got: {rendered}"
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

    let index = table
        .column_index(db_string("count"))
        .expect("count column");
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
