//! End-to-end coverage for HNSW config arguments on vector built-ins.

use selene_core::{
    GraphId, HnswIndexConfig, IStr, LabelSet, PropertyMap, Value, VectorValue, intern,
};
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

#[test]
fn drop_vector_index_removes_the_index_through_the_funnel() {
    let graph = graph(330_014);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 3)",
            &registry,
        )
        .expect("vector index creation executes");
    session
        .execute_source(
            "CALL selene.drop_vector_index('VectorDoc', 'embedding')",
            &registry,
        )
        .expect("vector index drop executes");

    assert_eq!(graph.read().vector_index_count(), 0);
}

#[test]
fn create_vector_index_can_register_explicit_hnsw_config() {
    let graph = graph(330_121);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = istr("VectorDoc");
    let embedding = istr("embedding");
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, Value::Vector(vector(&[1.0, 0.0, 0.0]))),
            )
            .expect("vector node inserts");
        txn.commit().expect("seed commits");
    }

    session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 3, 'hnsw', NULL, 'cosine', 24, 128)",
            &registry,
        )
        .expect("configured hnsw vector index creation executes");

    let snapshot = graph.read();
    assert_eq!(
        snapshot
            .vector_index_for(&doc, &embedding)
            .expect("configured vector index is committed")
            .hnsw_config(),
        Some(HnswIndexConfig::new(24, 128))
    );
    drop(snapshot);

    let table = execute_rows(&mut session, "SHOW INDEXES", &registry);
    assert_eq!(
        string_column(&table, "kind"),
        vec!["vector_hnsw_cosine(3,m=24,ef_construction=128)"]
    );
}

#[test]
fn create_vector_index_rejects_hnsw_config_for_flat_indexes() {
    let graph = graph(330_122);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let err = session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 3, 'flat', NULL, NULL, 24, 128)",
            &registry,
        )
        .expect_err("flat vector index must reject HNSW config");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("flat vector indexes do not accept HNSW config")
    ));
}
