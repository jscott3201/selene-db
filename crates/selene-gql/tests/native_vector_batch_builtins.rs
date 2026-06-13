//! End-to-end coverage for batched native vector-search built-ins.

use selene_core::{DbString, GraphId, LabelSet, NodeId, PropertyMap, Value, VectorValue};
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

fn seed_vector_graph(graph: &SharedGraph) {
    let doc = db_string("VectorDoc");
    let embedding = db_string("embedding");
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for i in 0..32 {
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[i as f32, 0.0]))),
                )
                .expect("vector node inserts");
        }
        txn.commit().expect("seed graph commits");
    }
}

fn seed_hnsw_vector_graph(graph: &SharedGraph, registry: &BuiltinProcedureRegistry) {
    seed_vector_graph(graph);
    let mut session = Session::new(graph);
    session
        .execute_source(
            "CALL selene.create_vector_index('VectorDoc', 'embedding', 2, 'hnsw')",
            registry,
        )
        .expect("hnsw vector index creation executes");
}

#[test]
fn vector_search_nodes_batch_groups_hits_by_query_index_without_ann_index() {
    let graph = graph(330_204);
    let registry = BuiltinProcedureRegistry::new();
    seed_vector_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("queries"),
        Value::List(vec![
            Value::Vector(vector(&[4.1, 0.0])),
            Value::Vector(vector(&[12.2, 0.0])),
        ]),
    );

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_batch('VectorDoc', 'embedding', $queries, 2, 'squared_euclidean') \
         YIELD query_index, node_id, distance",
        &registry,
    );

    assert_eq!(uint_column(&table, "query_index"), vec![0, 0, 1, 1]);
    let nodes = node_column(&table, "node_id");
    assert_eq!(nodes[0], NodeId::new(5));
    assert_eq!(nodes[2], NodeId::new(13));
}

#[test]
fn vector_search_nodes_batch_returns_no_rows_for_empty_query_list() {
    let graph = graph(330_205);
    let registry = BuiltinProcedureRegistry::new();
    seed_vector_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("queries"), Value::List(Vec::new()));

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_batch('VectorDoc', 'embedding', $queries, 2) \
         YIELD query_index, node_id, distance",
        &registry,
    );

    assert_eq!(table.row_count(), 0);
}

#[test]
fn vector_search_nodes_batch_rejects_mixed_query_dimensions() {
    let graph = graph(330_206);
    let registry = BuiltinProcedureRegistry::new();
    seed_vector_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("queries"),
        Value::List(vec![
            Value::Vector(vector(&[0.0, 0.0])),
            Value::Vector(vector(&[0.0, 0.0, 0.0])),
        ]),
    );

    let err = session
        .execute_source(
            "CALL selene.vector_search_nodes_batch('VectorDoc', 'embedding', $queries, 10)",
            &registry,
        )
        .expect_err("mixed query dimensions must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("same VECTOR dimension")
    ));
}

#[test]
fn vector_search_nodes_ann_batch_groups_hits_by_query_index() {
    let graph = graph(330_201);
    let registry = BuiltinProcedureRegistry::new();
    seed_hnsw_vector_graph(&graph, &registry);
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("queries"),
        Value::List(vec![
            Value::Vector(vector(&[4.1, 0.0])),
            Value::Vector(vector(&[12.2, 0.0])),
        ]),
    );

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_ann_batch('VectorDoc', 'embedding', $queries, 2, 'squared_euclidean', 32) \
         YIELD query_index, node_id, distance",
        &registry,
    );

    assert_eq!(uint_column(&table, "query_index"), vec![0, 0, 1, 1]);
    let nodes = node_column(&table, "node_id");
    assert_eq!(nodes[0], NodeId::new(5));
    assert_eq!(nodes[2], NodeId::new(13));
}

#[test]
fn vector_search_nodes_ann_batch_rejects_missing_ann_index() {
    let graph = graph(330_202);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("queries"),
        Value::List(vec![Value::Vector(vector(&[0.0, 0.0]))]),
    );

    let err = session
        .execute_source(
            "CALL selene.vector_search_nodes_ann_batch('VectorDoc', 'embedding', $queries, 10)",
            &registry,
        )
        .expect_err("missing ann index must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("requires a matching ANN vector index")
    ));
}

#[test]
fn vector_search_nodes_ann_batch_rejects_mixed_query_dimensions() {
    let graph = graph(330_203);
    let registry = BuiltinProcedureRegistry::new();
    seed_hnsw_vector_graph(&graph, &registry);
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("queries"),
        Value::List(vec![
            Value::Vector(vector(&[0.0, 0.0])),
            Value::Vector(vector(&[0.0, 0.0, 0.0])),
        ]),
    );

    let err = session
        .execute_source(
            "CALL selene.vector_search_nodes_ann_batch('VectorDoc', 'embedding', $queries, 10)",
            &registry,
        )
        .expect_err("mixed query dimensions must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("same VECTOR dimension")
    ));
}
