//! End-to-end coverage for the relocated `selene.*` platform built-ins.
//!
//! These tests drive `CALL selene.*` through the concrete native
//! [`BuiltinProcedureRegistry`] (STEP 3 relocation, no procedure-pack
//! machinery), exercising plan-time lookup, tier-checked dispatch, and — for the
//! mutation-tier built-ins — the single mutation funnel (`SHOW INDEXES` reads
//! the committed `property_index`, which the procedure populated through
//! `MutationContext::mutator`). They are the parity guard that the relocated
//! built-ins behave identically to the pack era with the registry swapped behind
//! the `ProcedureRegistry` trait, plus coverage for new native platform
//! built-ins added after the pack teardown.

use selene_core::{DbString, GraphId, Value};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ProcedureRegistry, Session, StatementOutput,
};
use selene_graph::SharedGraph;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
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

fn uint_column(table: &BindingTable, name: &str) -> Vec<u64> {
    let index = table
        .column_index(db_string(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::Uint(value)) => *value,
            other => panic!("expected uint in {name}, got {other:?}"),
        })
        .collect()
}

#[test]
fn show_procedures_lists_all_fifty_four_procedures() {
    let graph = graph(330_001);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let table = execute_rows(&mut session, "SHOW PROCEDURES", &registry);
    let names = string_column(&table, "name");

    assert_eq!(
        table.row_count(),
        54,
        "19 algo procedures + 35 platform built-ins"
    );
    for expected in [
        "selene.health",
        "selene.feature_status",
        "selene.verify",
        "selene.create_index",
        "selene.drop_index",
        "selene.vector_search_nodes",
        "selene.vector_search_nodes_batch",
        "selene.vector_score_nodes",
        "selene.vector_score_nodes_batch",
        "selene.vector_score_neighbors",
        "selene.vector_score_neighbors_batch",
        "selene.vector_score_expanded_candidates",
        "selene.vector_score_expanded_candidates_batch",
        "selene.vector_search_nodes_ann",
        "selene.vector_search_nodes_ann_batch",
        "selene.vector_search_expanded_candidates_ann",
        "selene.vector_search_candidate_state_expanded_ann",
        "selene.vector_search_expanded_candidates_ann_batch",
        "selene.vector_score_candidate_state",
        "selene.vector_score_candidate_state_nodes",
        "selene.vector_score_candidate_state_expanded",
        "selene.vector_score_candidate_state_expanded_batch",
        "selene.vector_candidate_states",
        "selene.vector_index_stats",
        "selene.text_index_stats",
        "selene.rebuild_vector_indexes",
        "selene.rebuild_recommended_vector_indexes",
        "selene.create_vector_index",
        "selene.drop_vector_index",
        "selene.create_text_index",
        "selene.drop_text_index",
        "selene.text_search_nodes",
        "selene.text_score_nodes",
        "selene.text_score_nodes_batch",
        "selene.text_score_candidate_state_expanded_batch",
        "algo.pagerank",
    ] {
        assert!(
            names.contains(&expected.to_owned()),
            "SHOW PROCEDURES must list {expected}"
        );
    }
    // `pack_history` is NOT relocated — it must not appear in the native registry.
    assert!(
        !names.contains(&"selene.pack_history".to_owned()),
        "pack_history must not be relocated into the native registry"
    );
}

#[test]
fn health_reports_node_and_edge_counts() {
    let graph = graph(330_002);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session
        .execute_source("INSERT (a:N)-[:E]->(b:N)", &registry)
        .expect("seed inserts");

    let table = execute_rows(
        &mut session,
        "CALL selene.health() YIELD graph_id, node_count, edge_count, schema_bound",
        &registry,
    );
    assert_eq!(table.row_count(), 1);
    assert_eq!(uint_column(&table, "node_count"), vec![2]);
    assert_eq!(uint_column(&table, "edge_count"), vec![1]);
    assert_eq!(uint_column(&table, "graph_id"), vec![330_002]);
}

#[test]
fn feature_status_reports_supported_rows() {
    let graph = graph(330_003);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let table = execute_rows(
        &mut session,
        "CALL selene.feature_status() YIELD feature_id, status, rationale",
        &registry,
    );
    let feature_ids = string_column(&table, "feature_id");
    let statuses = string_column(&table, "status");

    assert!(!feature_ids.is_empty());
    let gp04 = feature_ids
        .iter()
        .position(|value| value == "GP04")
        .expect("GP04 row exists");
    assert_eq!(statuses[gp04], "supported");
}

#[test]
fn verify_reports_ok_for_a_consistent_graph() {
    let graph = graph(330_004);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session
        .execute_source("INSERT (a:N)-[:E]->(b:N)", &registry)
        .expect("seed inserts");

    let table = execute_rows(
        &mut session,
        "CALL selene.verify() YIELD check, status, detail",
        &registry,
    );
    let statuses = string_column(&table, "status");
    assert!(
        !statuses.is_empty(),
        "verify emits at least the shallow checks"
    );
    assert!(
        statuses.iter().all(|status| status == "ok"),
        "a freshly inserted graph must verify clean: {statuses:?}"
    );
}

#[test]
fn verify_deep_runs_additional_checks() {
    let graph = graph(330_005);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session
        .execute_source("INSERT (a:N)-[:E]->(b:N)", &registry)
        .expect("seed inserts");

    let shallow = execute_rows(
        &mut session,
        "CALL selene.verify(false) YIELD check",
        &registry,
    );
    let deep = execute_rows(
        &mut session,
        "CALL selene.verify(true) YIELD check",
        &registry,
    );
    assert!(
        deep.row_count() > shallow.row_count(),
        "deep verify runs more checks than shallow"
    );
}

#[test]
fn create_index_then_show_indexes_confirms_funnel_commit() {
    let graph = graph(330_006);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    // The index DDL is performed by the mutation-tier built-in through the
    // `Mutator` funnel; SHOW INDEXES reads the COMMITTED graph's property index,
    // proving the create routed through the funnel and committed.
    session
        .execute_source(
            "CALL selene.create_index('Sensor', 'timestamp', 'i64')",
            &registry,
        )
        .expect("index creation executes");

    let table = execute_rows(&mut session, "SHOW INDEXES", &registry);
    assert_eq!(string_column(&table, "label"), vec!["Sensor"]);
    assert_eq!(string_column(&table, "property"), vec!["timestamp"]);
    assert_eq!(string_column(&table, "kind"), vec!["i64"]);
}

#[test]
fn drop_index_removes_the_index_through_the_funnel() {
    let graph = graph(330_007);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    session
        .execute_source(
            "CALL selene.create_index('Sensor', 'timestamp', 'i64')",
            &registry,
        )
        .expect("index creation executes");
    session
        .execute_source("CALL selene.drop_index('Sensor', 'timestamp')", &registry)
        .expect("index drop executes");

    let table = execute_rows(&mut session, "SHOW INDEXES", &registry);
    assert_eq!(table.row_count(), 0, "dropped index must not be listed");
}

#[test]
fn create_index_duplicate_is_an_invalid_argument_error() {
    let graph = graph(330_008);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    session
        .execute_source(
            "CALL selene.create_index('Sensor', 'timestamp', 'i64')",
            &registry,
        )
        .expect("first index creation executes");
    let err = session
        .execute_source(
            "CALL selene.create_index('Sensor', 'timestamp', 'i64')",
            &registry,
        )
        .expect_err("duplicate index must error");
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("already exists"),
        "duplicate index error should mention existence, got: {rendered}"
    );
}

#[test]
fn create_index_unknown_kind_is_rejected() {
    let graph = graph(330_009);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let err = session
        .execute_source(
            "CALL selene.create_index('Sensor', 'timestamp', 'not_a_kind')",
            &registry,
        )
        .expect_err("unknown index kind must error");
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("unknown index kind"),
        "error should name the bad kind, got: {rendered}"
    );
}

#[test]
fn unknown_selene_builtin_is_rejected_at_plan_time() {
    let graph = graph(330_010);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let err = session
        .execute_source("CALL selene.does_not_exist()", &registry)
        .expect_err("unknown built-in must be rejected");
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("does_not_exist") || rendered.to_lowercase().contains("procedure"),
        "unknown built-in error should name the procedure, got: {rendered}"
    );
}
