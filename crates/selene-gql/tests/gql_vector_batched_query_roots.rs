//! End-to-end coverage for batched vector scoring over GQL-produced roots.

use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, VectorValue, intern};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ProcedureRegistry, Session, StatementOutput,
};
use selene_graph::SharedGraph;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn vector(components: &[f32]) -> VectorValue {
    VectorValue::new(components.to_vec()).expect("test vector is valid")
}

fn props(entries: impl IntoIterator<Item = (IStr, Value)>) -> PropertyMap {
    PropertyMap::from_pairs(entries).expect("test property map is valid")
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
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

#[test]
fn vector_score_expanded_candidates_batch_accepts_gql_query_roots() {
    let graph = SharedGraph::new(GraphId::new(330_224));
    let registry = BuiltinProcedureRegistry::new();
    let (root_a, root_b, near_a, near_b) = seed_batched_query_root_graph(&graph);
    let mut session = Session::new(&graph);

    let table = execute_rows(
        &mut session,
        "MATCH (anchor:QueryAnchor)-[:DEPENDS_ON]->(root:VectorRoot) \
         WITH anchor.query_index AS query_index, anchor.query AS query, collect_list(root) AS roots \
         GROUP BY anchor.query_index, anchor.query \
         ORDER BY query_index \
         WITH collect_list(query) AS queries, collect_list(roots) AS root_sets \
         CALL selene.vector_score_expanded_candidates_batch('embedding', queries, root_sets, 'SUPPORTS', 2) \
         YIELD query_index, node_id, distance \
         RETURN query_index, node_id, distance",
        &registry,
    );

    assert_eq!(uint_column(&table, "query_index"), vec![0, 0, 1, 1]);
    assert_eq!(
        node_column(&table, "node_id"),
        vec![near_a, root_a, near_b, root_b]
    );
}

fn seed_batched_query_root_graph(graph: &SharedGraph) -> (NodeId, NodeId, NodeId, NodeId) {
    let root = istr("VectorRoot");
    let doc = istr("VectorDoc");
    let query_anchor = istr("QueryAnchor");
    let embedding = istr("embedding");
    let query = istr("query");
    let query_index = istr("query_index");
    let depends = istr("DEPENDS_ON");
    let supports = istr("SUPPORTS");
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    let root_labels = || LabelSet::from_iter([doc.clone(), root.clone()]);
    let root_a = mutator
        .create_node(
            root_labels(),
            props([(embedding.clone(), Value::Vector(vector(&[2.0, 0.0])))]),
        )
        .expect("root_a inserts");
    let root_b = mutator
        .create_node(
            root_labels(),
            props([(embedding.clone(), Value::Vector(vector(&[8.0, 0.0])))]),
        )
        .expect("root_b inserts");
    let near_a = mutator
        .create_node(
            LabelSet::single(doc.clone()),
            props([(embedding.clone(), Value::Vector(vector(&[3.0, 0.0])))]),
        )
        .expect("near_a inserts");
    let near_b = mutator
        .create_node(
            LabelSet::single(doc),
            props([(embedding, Value::Vector(vector(&[9.0, 0.0])))]),
        )
        .expect("near_b inserts");
    for (index, query_vector, root_node) in [
        (0, vector(&[3.2, 0.0]), root_a),
        (1, vector(&[9.1, 0.0]), root_b),
    ] {
        let anchor = mutator
            .create_node(
                LabelSet::single(query_anchor.clone()),
                props([
                    (query_index.clone(), Value::Int(index)),
                    (query.clone(), Value::Vector(query_vector)),
                ]),
            )
            .expect("query anchor inserts");
        mutator
            .create_edge(depends.clone(), anchor, root_node, PropertyMap::new())
            .expect("query dependency edge inserts");
    }
    mutator
        .create_edge(supports.clone(), root_a, near_a, PropertyMap::new())
        .expect("root_a support edge inserts");
    mutator
        .create_edge(supports, root_b, near_b, PropertyMap::new())
        .expect("root_b support edge inserts");
    txn.commit().expect("seed graph commits");
    (root_a, root_b, near_a, near_b)
}
