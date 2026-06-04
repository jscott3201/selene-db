//! End-to-end coverage for native vector candidate-scoring built-ins.

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

fn seed_vector_graph(graph: &SharedGraph) -> Vec<NodeId> {
    let doc = istr("VectorDoc");
    let embedding = istr("embedding");
    let other = istr("other");
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    let mut ids = Vec::new();
    for i in 0..8 {
        ids.push(
            mutator
                .create_node(
                    LabelSet::single(doc.clone()),
                    props(&embedding, Value::Vector(vector(&[i as f32, 0.0]))),
                )
                .expect("vector node inserts"),
        );
    }
    ids.push(
        mutator
            .create_node(
                LabelSet::single(doc),
                props(&other, Value::String(istr("not-a-vector"))),
            )
            .expect("non-vector node inserts"),
    );
    txn.commit().expect("seed graph commits");
    ids
}

#[test]
fn vector_score_nodes_reranks_explicit_candidates_without_index() {
    let graph = graph(330_207);
    let registry = BuiltinProcedureRegistry::new();
    let ids = seed_vector_graph(&graph);
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .delete_node(ids[7])
            .expect("delete candidate succeeds");
        txn.commit().expect("delete commits");
    }
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[3.2, 0.0])));
    session.bind_parameter(
        istr("nodes"),
        Value::List(vec![
            Value::NodeRef(ids[5]),
            Value::NodeRef(ids[3]),
            Value::NodeRef(ids[3]),
            Value::NodeRef(ids[0]),
            Value::NodeRef(ids[7]),
            Value::NodeRef(ids[8]),
            Value::NodeRef(NodeId::new(999)),
        ]),
    );

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_score_nodes('embedding', $query, $nodes, 3, 'squared_euclidean') \
         YIELD node_id, distance",
        &registry,
    );

    assert_eq!(node_column(&table, "node_id"), vec![ids[3], ids[5], ids[0]]);
}

#[test]
fn vector_score_nodes_returns_no_rows_for_empty_candidate_list() {
    let graph = graph(330_208);
    let registry = BuiltinProcedureRegistry::new();
    seed_vector_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));
    session.bind_parameter(istr("nodes"), Value::List(Vec::new()));

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_score_nodes('embedding', $query, $nodes, 10) \
         YIELD node_id, distance",
        &registry,
    );

    assert_eq!(table.row_count(), 0);
}

#[test]
fn vector_score_nodes_rejects_non_node_candidates() {
    let graph = graph(330_209);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));
    session.bind_parameter(istr("nodes"), Value::List(vec![Value::Int(1)]));

    let err = session
        .execute_source(
            "CALL selene.vector_score_nodes('embedding', $query, $nodes, 10)",
            &registry,
        )
        .expect_err("non-node candidates must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("nodes[0] must be a NODE")
    ));
}
