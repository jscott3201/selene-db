//! BRIEF-118 GQL surface coverage.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use selene_algorithms_pack::AlgorithmsPack;
use selene_core::{GraphId, IStr, Value, intern};
use selene_gql::{
    BindingTable, EmptyProcedureRegistry, ProcedureContext, ProcedureError, ProcedureHandle,
    ProcedureMetadata, ProcedureMutability, ProcedureOutputSchema, ProcedureRegistry,
    ProcedureResult, ProcedureSignature, ProcedureTier, Session, StatementOutput, parse,
};
use selene_graph::{IndexProvider, SharedGraph};
use selene_pack::ProcedurePackRegistry;
use selene_vector::{HnswConfig, HnswIndexRegistry, IvfConfig, IvfIndexRegistry};
use selene_vector_pack::VectorPack;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn graph_with_vector_providers(id: u64) -> SharedGraph {
    let hnsw = Arc::new(
        HnswIndexRegistry::new(HnswConfig::new(2).expect("HNSW config builds"))
            .expect("HNSW provider builds"),
    );
    let ivf = Arc::new(
        IvfIndexRegistry::new(IvfConfig::new(2).expect("IVF config builds"))
            .expect("IVF provider builds"),
    );
    SharedGraph::builder(GraphId::new(id))
        .with_provider(hnsw as Arc<dyn IndexProvider>)
        .with_provider(ivf as Arc<dyn IndexProvider>)
        .build()
        .expect("graph with vector providers builds")
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
        .column_index(istr(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::String(value)) => value.as_str().to_owned(),
            Some(Value::ExternalString(value)) => value.as_ref().to_owned(),
            other => panic!("expected string in {name}, got {other:?}"),
        })
        .collect()
}

fn full_registry() -> ProcedurePackRegistry {
    let vector = VectorPack::new();
    let algorithms = AlgorithmsPack::new();
    ProcedurePackRegistry::builder()
        .with_builtins()
        .with_external_pack(vector.external_pack())
        .with_external_pack(algorithms.external_pack())
        .build()
        .expect("full default registry builds")
}

#[test]
fn show_indexes_lists_property_indexes_only() {
    let graph = graph(118_001);
    let registry = ProcedurePackRegistry::with_builtins().expect("built-ins register");
    let mut session = Session::new(&graph);

    session
        .execute_source(
            "CALL selene.create_index('Sensor', 'timestamp', 'i64')",
            &registry,
        )
        .expect("index creation executes");
    let table = execute_rows(&mut session, "SHOW INDEXES", &registry);

    assert_eq!(column_strings(&table, "label"), vec!["Sensor"]);
    assert_eq!(column_strings(&table, "property"), vec!["timestamp"]);
    assert_eq!(column_strings(&table, "kind"), vec!["i64"]);
}

#[test]
fn show_indexes_does_not_report_vector_indexes() {
    let graph = graph_with_vector_providers(118_002);
    let registry = full_registry();
    let mut session = Session::new(&graph);

    session
        .execute_source(
            "CALL vector.create_index('episodes', 'hnsw', {dim: 2})",
            &registry,
        )
        .expect("vector index creation executes");
    let vector_table = execute_rows(
        &mut session,
        "CALL vector.list_indexes() YIELD name, kind, dim",
        &registry,
    );
    assert!(column_strings(&vector_table, "name").contains(&"episodes".to_owned()));

    let table = execute_rows(&mut session, "SHOW INDEXES", &registry);

    assert!(table.rows().is_empty());
}

#[test]
fn show_procedures_lists_default_pack_registry() {
    let graph = graph(118_003);
    let registry = full_registry();
    let mut session = Session::new(&graph);
    let table = execute_rows(&mut session, "SHOW PROCEDURES", &registry);
    let names = column_strings(&table, "name");

    assert_eq!(table.row_count(), 35);
    assert!(names.contains(&"selene.feature_status".to_owned()));
    assert!(names.contains(&"vector.list_indexes".to_owned()));
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
    let registry = ProcedurePackRegistry::with_builtins().expect("built-ins register");
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
fn create_index_rejection_names_follow_up_storage_dependency() {
    let error = parse("CREATE INDEX sensor_ts ON :Sensor(timestamp)")
        .expect_err("named CREATE INDEX is still rejected");

    assert!(
        error
            .to_string()
            .contains("storage-layer named-index support")
    );
}

struct CountingRegistry {
    counter: Arc<AtomicUsize>,
}

impl ProcedureRegistry for CountingRegistry {
    fn lookup(&self, name: &[IStr]) -> Option<ProcedureMetadata> {
        (name == [istr("test"), istr("bump")]).then(|| ProcedureMetadata {
            handle: ProcedureHandle::new(1),
            signature: ProcedureSignature::default(),
            output_schema: ProcedureOutputSchema::default(),
            tier: ProcedureTier::Graph,
            mutability: ProcedureMutability::Read,
            capability_required: None,
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
