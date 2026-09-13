//! BRIEF-118 GQL surface coverage.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use selene_core::{DbString, GraphId, LabelSet, PropertyMap, Value};
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

fn column_uints(table: &BindingTable, name: &str) -> Vec<u64> {
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

fn column_bools(table: &BindingTable, name: &str) -> Vec<bool> {
    let index = table
        .column_index(db_string(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::Bool(value)) => *value,
            other => panic!("expected bool in {name}, got {other:?}"),
        })
        .collect()
}

fn full_registry() -> BuiltinProcedureRegistry {
    BuiltinProcedureRegistry::new()
}

fn create_deleted_row_pressure(graph: &SharedGraph) {
    let label = db_string("CompactionSurfaceNode");
    let edge = db_string("COMPACTION_SURFACE_EDGE");
    let mut txn = graph.begin_write();
    {
        let mut mutator = txn.mutator();
        let a = mutator
            .create_node(LabelSet::single(label.clone()), PropertyMap::new())
            .expect("create a");
        let b = mutator
            .create_node(LabelSet::single(label.clone()), PropertyMap::new())
            .expect("create b");
        let c = mutator
            .create_node(LabelSet::single(label), PropertyMap::new())
            .expect("create c");
        mutator
            .create_edge(edge.clone(), a, b, PropertyMap::new())
            .expect("create edge a->b");
        mutator
            .create_edge(edge, b, c, PropertyMap::new())
            .expect("create edge b->c");
        mutator.delete_node(b).expect("delete middle node");
    }
    txn.commit().expect("deleted-row fixture commits");
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

    assert_eq!(table.row_count(), 69);
    assert!(names.contains(&"selene.compaction_stats".to_owned()));
    assert!(names.contains(&"selene.feature_status".to_owned()));
    assert!(names.contains(&"selene.verify".to_owned()));
    assert!(names.contains(&"selene.vector_search_nodes".to_owned()));
    assert!(names.contains(&"selene.vector_search_nodes_batch".to_owned()));
    assert!(names.contains(&"selene.vector_score_nodes".to_owned()));
    assert!(names.contains(&"selene.vector_score_nodes_batch".to_owned()));
    assert!(names.contains(&"selene.text_search_nodes".to_owned()));
    assert!(names.contains(&"selene.json_contains_nodes".to_owned()));
    assert!(names.contains(&"selene.json_path_exists_nodes".to_owned()));
    assert!(names.contains(&"selene.json_path_contains_nodes".to_owned()));
    assert!(names.contains(&"selene.json_path_value_nodes".to_owned()));
    assert!(names.contains(&"selene.json_contains_candidate_nodes".to_owned()));
    assert!(names.contains(&"selene.json_path_exists_candidate_nodes".to_owned()));
    assert!(names.contains(&"selene.json_path_contains_candidate_nodes".to_owned()));
    assert!(names.contains(&"selene.json_path_value_candidate_nodes".to_owned()));
    assert!(names.contains(&"selene.text_score_nodes".to_owned()));
    assert!(names.contains(&"selene.text_score_nodes_batch".to_owned()));
    assert!(names.contains(&"selene.text_score_candidate_state".to_owned()));
    assert!(names.contains(&"selene.text_score_candidate_state_nodes".to_owned()));
    assert!(names.contains(&"selene.text_score_candidate_state_expanded_batch".to_owned()));
    assert!(names.contains(&"selene.reciprocal_rank_fusion".to_owned()));
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
    assert!(names.contains(&"selene.reachable_nodes".to_owned()));
    assert!(names.contains(&"selene.vector_index_stats".to_owned()));
    assert!(names.contains(&"selene.text_index_stats".to_owned()));
    assert!(names.contains(&"selene.rebuild_vector_indexes".to_owned()));
    assert!(names.contains(&"selene.rebuild_recommended_vector_indexes".to_owned()));
    assert!(names.contains(&"selene.compact".to_owned()));
    assert!(names.contains(&"selene.create_vector_index".to_owned()));
    assert!(names.contains(&"selene.drop_vector_index".to_owned()));
    assert!(names.contains(&"selene.create_text_index".to_owned()));
    assert!(names.contains(&"selene.drop_text_index".to_owned()));
    assert!(names.contains(&"algo.pagerank".to_owned()));
}

#[test]
fn compaction_procedures_execute_through_call_surface() {
    let graph = graph(118_007);
    create_deleted_row_pressure(&graph);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let before = execute_rows(
        &mut session,
        "CALL selene.compaction_stats() \
         YIELD reclaimable_rows, reclaimable_row_basis_points, \
               compaction_recommended, dense",
        &registry,
    );
    assert_eq!(column_uints(&before, "reclaimable_rows"), vec![3]);
    assert_eq!(
        column_uints(&before, "reclaimable_row_basis_points"),
        vec![6_000]
    );
    assert_eq!(column_bools(&before, "compaction_recommended"), vec![false]);
    assert_eq!(column_bools(&before, "dense"), vec![false]);

    let compact = execute_rows(
        &mut session,
        "CALL selene.compact() \
         YIELD before_reclaimable_rows, before_reclaimable_row_basis_points, \
               before_compaction_recommended, reclaimed_nodes, reclaimed_edges, \
               after_reclaimable_rows, after_reclaimable_row_basis_points, \
               after_compaction_recommended, after_dense",
        &registry,
    );
    assert_eq!(column_uints(&compact, "before_reclaimable_rows"), vec![3]);
    assert_eq!(
        column_uints(&compact, "before_reclaimable_row_basis_points"),
        vec![6_000]
    );
    assert_eq!(
        column_bools(&compact, "before_compaction_recommended"),
        vec![false]
    );
    assert_eq!(column_uints(&compact, "reclaimed_nodes"), vec![1]);
    assert_eq!(column_uints(&compact, "reclaimed_edges"), vec![2]);
    assert_eq!(column_uints(&compact, "after_reclaimable_rows"), vec![0]);
    assert_eq!(
        column_uints(&compact, "after_reclaimable_row_basis_points"),
        vec![0]
    );
    assert_eq!(
        column_bools(&compact, "after_compaction_recommended"),
        vec![false]
    );
    assert_eq!(column_bools(&compact, "after_dense"), vec![true]);

    let after = execute_rows(
        &mut session,
        "CALL selene.compaction_stats() \
         YIELD reclaimable_rows, reclaimable_row_basis_points, \
               compaction_recommended, dense",
        &registry,
    );
    assert_eq!(column_uints(&after, "reclaimable_rows"), vec![0]);
    assert_eq!(
        column_uints(&after, "reclaimable_row_basis_points"),
        vec![0]
    );
    assert_eq!(column_bools(&after, "compaction_recommended"), vec![false]);
    assert_eq!(column_bools(&after, "dense"), vec![true]);
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
fn explain_keyword_accepts_comment_boundary() {
    let graph = graph(118_006);
    let mut session = Session::new(&graph);
    let table = execute_rows(
        &mut session,
        "EXPLAIN /* c */ MATCH (n:Sensor) RETURN n.id",
        &EmptyProcedureRegistry,
    );
    let plan = column_strings(&table, "plan");

    assert!(plan[0].contains("Project"));
    assert!(plan[0].contains("Scan"));
}

#[test]
fn explain_keyword_requires_boundary_before_statement_head() {
    for source in [
        "EXPLAINMATCH (n) RETURN n",
        "EXPLAINRETURN 1",
        "EXPLAINCALL test.bump()",
        "EXPLAINCREATE NODE TYPE :Thing ()",
    ] {
        assert!(parse(source).is_err(), "{source} should parse-reject");
    }
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
    let graph = graph(118_007);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let table = execute_rows(
        &mut session,
        "CALL selene.feature_status() YIELD feature_id, status, rationale, feature_name, surface, profile_relation, claim_state, evidence_status, evidence_count, profile_hash",
        &registry,
    );
    let feature_ids = column_strings(&table, "feature_id");
    let statuses = column_strings(&table, "status");
    let hashes = column_strings(&table, "profile_hash");

    assert!(!feature_ids.is_empty());
    let gp04 = feature_ids
        .iter()
        .position(|value| value == "GP04")
        .expect("GP04 row exists");
    assert_eq!(statuses[gp04], "supported");
    assert!(
        hashes
            .iter()
            .all(|hash| hash == selene_profile::PROFILE_HASH)
    );
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
