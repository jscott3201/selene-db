//! BRIEF-118 GQL surface coverage.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use selene_core::{DbString, GraphId, Value};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, CatalogOp, EmptyProcedureRegistry, PipelineOp,
    ProcedureContext, ProcedureError, ProcedureHandle, ProcedureMetadata, ProcedureMutability,
    ProcedureOutputSchema, ProcedureRegistry, ProcedureResult, ProcedureSignature, ProcedureTier,
    Session, StatementOutput, analyze, parse, plan,
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

fn column_strings(table: &BindingTable, name: &str) -> Vec<String> {
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

fn full_registry() -> BuiltinProcedureRegistry {
    BuiltinProcedureRegistry::new()
}

#[test]
fn show_indexes_lists_registered_indexes() {
    let graph = graph(118_001);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    session
        .execute_source(
            "CALL selene.create_index('Sensor', 'timestamp', 'i64')",
            &registry,
        )
        .expect("index creation executes");
    let table = execute_rows(&mut session, "SHOW INDEXES", &registry);

    assert_eq!(
        column_strings(&table, "name"),
        vec!["idx:6:Sensor:9:timestamp"]
    );
    assert_eq!(column_strings(&table, "label"), vec!["Sensor"]);
    assert_eq!(column_strings(&table, "property"), vec!["timestamp"]);
    assert_eq!(column_strings(&table, "kind"), vec!["i64"]);
}

#[test]
fn show_procedures_lists_default_registry() {
    let graph = graph(118_003);
    let registry = full_registry();
    let mut session = Session::new(&graph);
    let table = execute_rows(&mut session, "SHOW PROCEDURES", &registry);
    let names = column_strings(&table, "name");

    assert_eq!(table.row_count(), 56);
    assert!(names.contains(&"selene.feature_status".to_owned()));
    assert!(names.contains(&"selene.verify".to_owned()));
    assert!(names.contains(&"selene.vector_search_nodes".to_owned()));
    assert!(names.contains(&"selene.vector_search_nodes_batch".to_owned()));
    assert!(names.contains(&"selene.vector_score_nodes".to_owned()));
    assert!(names.contains(&"selene.vector_score_nodes_batch".to_owned()));
    assert!(names.contains(&"selene.text_search_nodes".to_owned()));
    assert!(names.contains(&"selene.json_contains_nodes".to_owned()));
    assert!(names.contains(&"selene.json_path_exists_nodes".to_owned()));
    assert!(names.contains(&"selene.text_score_nodes".to_owned()));
    assert!(names.contains(&"selene.text_score_nodes_batch".to_owned()));
    assert!(names.contains(&"selene.text_score_candidate_state_expanded_batch".to_owned()));
    assert!(names.contains(&"selene.vector_score_neighbors".to_owned()));
    assert!(names.contains(&"selene.vector_score_neighbors_batch".to_owned()));
    assert!(names.contains(&"selene.vector_score_expanded_candidates".to_owned()));
    assert!(names.contains(&"selene.vector_score_expanded_candidates_batch".to_owned()));
    assert!(names.contains(&"selene.vector_search_nodes_ann".to_owned()));
    assert!(names.contains(&"selene.vector_search_nodes_ann_batch".to_owned()));
    assert!(names.contains(&"selene.vector_search_expanded_candidates_ann".to_owned()));
    assert!(names.contains(&"selene.vector_search_candidate_state_expanded_ann".to_owned()));
    assert!(names.contains(&"selene.vector_search_expanded_candidates_ann_batch".to_owned()));
    assert!(names.contains(&"selene.vector_score_candidate_state".to_owned()));
    assert!(names.contains(&"selene.vector_score_candidate_state_nodes".to_owned()));
    assert!(names.contains(&"selene.vector_score_candidate_state_expanded".to_owned()));
    assert!(names.contains(&"selene.vector_score_candidate_state_expanded_batch".to_owned()));
    assert!(names.contains(&"selene.vector_candidate_states".to_owned()));
    assert!(names.contains(&"selene.vector_index_stats".to_owned()));
    assert!(names.contains(&"selene.text_index_stats".to_owned()));
    assert!(names.contains(&"selene.rebuild_vector_indexes".to_owned()));
    assert!(names.contains(&"selene.rebuild_recommended_vector_indexes".to_owned()));
    assert!(names.contains(&"selene.create_vector_index".to_owned()));
    assert!(names.contains(&"selene.drop_vector_index".to_owned()));
    assert!(names.contains(&"selene.create_text_index".to_owned()));
    assert!(names.contains(&"selene.drop_text_index".to_owned()));
    assert!(names.contains(&"algo.pagerank".to_owned()));
}

#[test]
fn explain_returns_plan_without_executing_inner_statement() {
    let graph = graph(118_004);
    let counter = Arc::new(AtomicUsize::new(0));
    let registry = CountingRegistry {
        counter: Arc::clone(&counter),
    };
    let mut session = Session::new(&graph);

    let table = execute_rows(&mut session, "EXPLAIN CALL test.bump()", &registry);
    let plan = column_strings(&table, "plan");

    assert_eq!(counter.load(Ordering::SeqCst), 0);
    assert_eq!(plan.len(), 1);
    assert!(plan[0].contains("Call"));
}

#[test]
fn explain_match_plan_mentions_scan_and_project() {
    let graph = graph(118_005);
    let mut session = Session::new(&graph);
    let table = execute_rows(
        &mut session,
        "EXPLAIN MATCH (n:Sensor) RETURN n.id",
        &EmptyProcedureRegistry,
    );
    let plan = column_strings(&table, "plan");

    assert!(plan[0].contains("Project"));
    assert!(plan[0].contains("Scan"));
}

#[test]
fn explain_rejects_transaction_control_and_nested_explain() {
    for source in [
        "EXPLAIN START TRANSACTION",
        "EXPLAIN COMMIT",
        "EXPLAIN ROLLBACK",
        "EXPLAIN EXPLAIN RETURN 1",
    ] {
        assert!(parse(source).is_err(), "{source} should parse-reject");
    }
}

#[test]
fn feature_status_procedure_returns_supported_rows() {
    let graph = graph(118_006);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let table = execute_rows(
        &mut session,
        "CALL selene.feature_status() YIELD feature_id, status, rationale",
        &registry,
    );
    let feature_ids = column_strings(&table, "feature_id");
    let statuses = column_strings(&table, "status");

    assert!(!feature_ids.is_empty());
    let gp04 = feature_ids
        .iter()
        .position(|value| value == "GP04")
        .expect("GP04 row exists");
    assert_eq!(statuses[gp04], "supported");
}

#[test]
fn create_index_surface_parses_and_preserves_planner_fields() {
    let statement = parse("CREATE INDEX sensor_ts ON :Sensor(timestamp)").expect("DDL parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("DDL analyzes");
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("DDL plans");
    let [
        PipelineOp::Catalog(CatalogOp::CreateIndex {
            name,
            label,
            properties,
            ..
        }),
    ] = plan.pipeline.as_slice()
    else {
        panic!("expected CREATE INDEX catalog op");
    };

    assert_eq!(name.as_str(), "sensor_ts");
    assert_eq!(label.as_str(), "Sensor");
    assert_eq!(
        properties
            .iter()
            .map(|property| property.as_str())
            .collect::<Vec<_>>(),
        ["timestamp"]
    );
}

struct CountingRegistry {
    counter: Arc<AtomicUsize>,
}

impl ProcedureRegistry for CountingRegistry {
    fn lookup(&self, name: &[DbString]) -> Option<ProcedureMetadata> {
        (name == [db_string("test"), db_string("bump")]).then(|| {
            ProcedureMetadata::new(
                ProcedureHandle::new(1),
                ProcedureSignature::default(),
                ProcedureOutputSchema::default(),
                ProcedureTier::Graph,
                ProcedureMutability::Read,
            )
        })
    }

    fn execute(
        &self,
        _handle: ProcedureHandle,
        _args: &[Value],
        _ctx: &mut ProcedureContext<'_, '_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Ok(ProcedureResult::default())
    }
}
