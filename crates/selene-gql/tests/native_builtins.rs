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

use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, VectorValue, intern};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ExecutorError, ProcedureError, ProcedureRegistry,
    Session, StatementOutput,
};
use selene_graph::SharedGraph;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn vector(components: &[f32]) -> VectorValue {
    VectorValue::new(components.to_vec()).expect("test vector is valid")
}

fn props(key: &IStr, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
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

fn uint_column(table: &BindingTable, name: &str) -> Vec<u64> {
    let index = table
        .column_index(istr(name))
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

fn node_column(table: &BindingTable, name: &str) -> Vec<NodeId> {
    let index = table
        .column_index(istr(name))
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

#[test]
fn show_procedures_lists_all_twenty_five_procedures() {
    let graph = graph(330_001);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let table = execute_rows(&mut session, "SHOW PROCEDURES", &registry);
    let names = string_column(&table, "name");

    assert_eq!(
        table.row_count(),
        25,
        "19 algo procedures + 6 platform built-ins"
    );
    for expected in [
        "selene.health",
        "selene.feature_status",
        "selene.verify",
        "selene.create_index",
        "selene.drop_index",
        "selene.vector_search_nodes",
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
fn vector_search_nodes_returns_exact_matches_from_vector_parameter() {
    let graph = graph(330_011);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("VectorDoc");
    let other = istr("VectorOther");
    let embedding = istr("embedding");

    {
        let mut txn = graph.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[2.0, 0.0]))),
                )
                .expect("first vector node inserts");
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[1.0, 0.0]))),
                )
                .expect("second vector node inserts");
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::String(istr("skip"))),
                )
                .expect("non-vector property node inserts");
            mutator
                .create_node(
                    LabelSet::single(other),
                    props(&embedding, Value::Vector(vector(&[0.0, 0.0]))),
                )
                .expect("other label vector node inserts");
        }
        txn.commit().expect("seed graph commits");
    }

    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));
    let table = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes('VectorDoc', 'embedding', $query, 10) \
         YIELD node_id, distance",
        &registry,
    );

    assert_eq!(
        node_column(&table, "node_id"),
        vec![NodeId::new(2), NodeId::new(1)]
    );
    assert_eq!(float_column(&table, "distance"), vec![1.0, 4.0]);
}

#[test]
fn vector_search_nodes_query_parameter_must_be_vector() {
    let graph = graph(330_012);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    session.bind_parameter(istr("query"), Value::List(vec![Value::Float(0.0)]));
    let err = session
        .execute_source(
            "CALL selene.vector_search_nodes('VectorDoc', 'embedding', $query, 10)",
            &registry,
        )
        .expect_err("non-vector query parameter must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("query must be a VECTOR")
    ));
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
