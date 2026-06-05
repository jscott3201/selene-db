//! End-to-end coverage for ANN-root graph-expanded vector search built-ins.

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

fn seed_ann_expansion_graph(shared: &SharedGraph) -> (NodeId, NodeId, NodeId, NodeId, NodeId) {
    let summary = istr("Summary");
    let fact = istr("Fact");
    let embedding = istr("embedding");
    let supports = istr("SUPPORTS");
    let mentions = istr("MENTIONS");
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let root_a = mutator
        .create_node(
            LabelSet::single(summary.clone()),
            props(&embedding, Value::Vector(vector(&[0.2, 0.0]))),
        )
        .expect("root a inserts");
    let fact_a = mutator
        .create_node(
            LabelSet::single(fact.clone()),
            props(&embedding, Value::Vector(vector(&[0.0, 0.0]))),
        )
        .expect("fact a inserts");
    let root_b = mutator
        .create_node(
            LabelSet::single(summary),
            props(&embedding, Value::Vector(vector(&[10.2, 0.0]))),
        )
        .expect("root b inserts");
    let fact_b = mutator
        .create_node(
            LabelSet::single(fact.clone()),
            props(&embedding, Value::Vector(vector(&[10.0, 0.0]))),
        )
        .expect("fact b inserts");
    let wrong_edge_fact = mutator
        .create_node(
            LabelSet::single(fact),
            props(&embedding, Value::Vector(vector(&[0.0, 0.0]))),
        )
        .expect("wrong-edge fact inserts");
    mutator
        .create_edge(supports.clone(), root_a, fact_a, PropertyMap::new())
        .expect("support edge a inserts");
    mutator
        .create_edge(supports, root_b, fact_b, PropertyMap::new())
        .expect("support edge b inserts");
    mutator
        .create_edge(mentions, root_a, wrong_edge_fact, PropertyMap::new())
        .expect("wrong edge inserts");
    txn.commit().expect("seed commits");
    (root_a, fact_a, root_b, fact_b, wrong_edge_fact)
}

fn create_hnsw_index(
    session: &mut Session<'_>,
    registry: &BuiltinProcedureRegistry,
    dimension: usize,
) {
    session
        .execute_source(
            &format!(
                "CALL selene.create_vector_index('Summary', 'embedding', {dimension}, 'hnsw')"
            ),
            registry,
        )
        .expect("hnsw vector index creation executes");
}

#[test]
fn vector_search_expanded_candidates_ann_uses_ann_roots_then_graph_rerank() {
    let graph = graph(330_231);
    let registry = BuiltinProcedureRegistry::new();
    let (_root_a, fact_a, _root_b, _fact_b, wrong_edge_fact) = seed_ann_expansion_graph(&graph);
    let mut session = Session::new(&graph);
    create_hnsw_index(&mut session, &registry, 2);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_search_expanded_candidates_ann(
            'Summary', 'embedding', $query, 1, 'SUPPORTS', 2,
            'outgoing', 'squared_euclidean', 32
         ) YIELD node_id, distance",
        &registry,
    );

    let nodes = node_column(&table, "node_id");
    assert_eq!(nodes[0], fact_a);
    assert!(!nodes.contains(&wrong_edge_fact));
    assert_eq!(table.row_count(), 2);
}

#[test]
fn vector_search_expanded_candidates_ann_batch_groups_hits_by_query_index() {
    let graph = graph(330_232);
    let registry = BuiltinProcedureRegistry::new();
    let (_root_a, fact_a, _root_b, fact_b, _wrong_edge_fact) = seed_ann_expansion_graph(&graph);
    let mut session = Session::new(&graph);
    create_hnsw_index(&mut session, &registry, 2);
    session.bind_parameter(
        istr("queries"),
        Value::List(vec![
            Value::Vector(vector(&[0.0, 0.0])),
            Value::Vector(vector(&[10.0, 0.0])),
        ]),
    );

    let table = execute_rows(
        &mut session,
        "CALL selene.vector_search_expanded_candidates_ann_batch(
            'Summary', 'embedding', $queries, 2, 'SUPPORTS', 1,
            'outgoing', 'squared_euclidean', 32
         ) YIELD query_index, node_id, distance",
        &registry,
    );

    assert_eq!(uint_column(&table, "query_index"), vec![0, 1]);
    assert_eq!(node_column(&table, "node_id"), vec![fact_a, fact_b]);
}

#[test]
fn vector_search_expanded_candidates_ann_requires_ann_index() {
    let graph = graph(330_233);
    let registry = BuiltinProcedureRegistry::new();
    seed_ann_expansion_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("query"), Value::Vector(vector(&[0.0, 0.0])));

    let err = session
        .execute_source(
            "CALL selene.vector_search_expanded_candidates_ann(
                'Summary', 'embedding', $query, 1, 'SUPPORTS', 1
             )",
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
fn vector_search_expanded_candidates_ann_batch_rejects_mixed_query_dimensions() {
    let graph = graph(330_234);
    let registry = BuiltinProcedureRegistry::new();
    seed_ann_expansion_graph(&graph);
    let mut session = Session::new(&graph);
    create_hnsw_index(&mut session, &registry, 2);
    session.bind_parameter(
        istr("queries"),
        Value::List(vec![
            Value::Vector(vector(&[0.0, 0.0])),
            Value::Vector(vector(&[0.0, 0.0, 0.0])),
        ]),
    );

    let err = session
        .execute_source(
            "CALL selene.vector_search_expanded_candidates_ann_batch(
                'Summary', 'embedding', $queries, 1, 'SUPPORTS', 1
             )",
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
