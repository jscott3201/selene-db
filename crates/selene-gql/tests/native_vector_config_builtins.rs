//! End-to-end coverage for HNSW config arguments on vector built-ins.

use selene_core::{
    DbString, GraphId, HnswIndexConfig, IvfIndexConfig, LabelSet, PropertyMap, Value, VectorValue,
};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ExecutorError, ProcedureError, ProcedureRegistry,
    Session, StatementOutput,
};
use selene_graph::SharedGraph;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn vector(components: &[f32]) -> VectorValue {
    VectorValue::new(components.to_vec()).expect("test vector is valid")
}

fn props(key: &DbString, value: Value) -> PropertyMap {
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
    let doc = db_string("VectorDoc");
    let embedding = db_string("embedding");
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
        } if detail.contains("only HNSW vector indexes accept HNSW config")
    ));
}

#[test]
fn create_vector_index_can_register_explicit_ivf_config() {
    let graph = graph(330_123);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let doc = db_string("VectorDoc");
    let embedding = db_string("embedding");
    {
        let mut txn = graph.begin_write();
        for idx in 0..6 {
            txn.mutator()
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[idx as f32, 0.0, 1.0]))),
                )
                .expect("vector node inserts");
        }
        txn.commit().expect("seed commits");
    }

    session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 3, 'ivf', NULL, 'cosine', NULL, NULL, 4)",
            &registry,
        )
        .expect("configured ivf vector index creation executes");

    let snapshot = graph.read();
    let index = snapshot
        .vector_index_for(&doc, &embedding)
        .expect("configured vector index is committed");
    assert_eq!(index.ivf_config(), Some(IvfIndexConfig::new(4)));
    assert_eq!(index.memory_usage().ivf_centroids, 4);
    drop(snapshot);

    let table = execute_rows(&mut session, "SHOW INDEXES", &registry);
    assert_eq!(
        string_column(&table, "kind"),
        vec!["vector_ivf_cosine(3,target_centroids=4)"]
    );
}

#[test]
fn create_vector_index_rejects_ivf_config_for_flat_indexes() {
    let graph = graph(330_124);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let err = session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 3, 'flat', NULL, NULL, NULL, NULL, 4)",
            &registry,
        )
        .expect_err("flat vector index must reject IVF config");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("only IVF vector indexes accept IVF config")
    ));
}

#[test]
fn create_vector_index_rejects_oversized_ivf_config() {
    let graph = graph(330_125);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let err = session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 3, 'ivf', NULL, NULL, NULL, NULL, 2048)",
            &registry,
        )
        .expect_err("oversized IVF config must be rejected");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("target_centroids exceeds engine cap")
    ));
}
