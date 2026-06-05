//! End-to-end coverage for batched BM25 scoring over GQL-produced candidates.

use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ProcedureRegistry, Session, StatementOutput,
};
use selene_graph::SharedGraph;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
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
fn text_score_nodes_batch_accepts_gql_query_candidate_sets() {
    let graph = SharedGraph::new(GraphId::new(330_225));
    let registry = BuiltinProcedureRegistry::new();
    let (graph_strong, graph_weak, memory_strong) = seed_batched_text_root_graph(&graph);
    graph
        .create_text_index(istr("TextDoc"), istr("body"))
        .expect("text index registers");
    let mut session = Session::new(&graph);

    let table = execute_rows(
        &mut session,
        "MATCH (anchor:TextQueryAnchor)-[:DEPENDS_ON]->(root:TextRoot) \
         MATCH (root)-[:SUPPORTS]->(candidate:TextDoc) \
         WITH anchor.query_index AS query_index, anchor.query_text AS query_text, collect_list(candidate) AS candidates \
         GROUP BY anchor.query_index, anchor.query_text \
         ORDER BY query_index \
         WITH collect_list(query_text) AS queries, collect_list(candidates) AS candidate_sets \
         CALL selene.text_score_nodes_batch('TextDoc', 'body', queries, candidate_sets, 2) \
         YIELD query_index, node_id, score \
         RETURN query_index, node_id, score",
        &registry,
    );

    assert_eq!(uint_column(&table, "query_index"), vec![0, 0, 1, 1]);
    assert_eq!(
        node_column(&table, "node_id"),
        vec![graph_strong, graph_weak, memory_strong, graph_strong]
    );
}

fn seed_batched_text_root_graph(graph: &SharedGraph) -> (NodeId, NodeId, NodeId) {
    let root = istr("TextRoot");
    let doc = istr("TextDoc");
    let query_anchor = istr("TextQueryAnchor");
    let body = istr("body");
    let query_text = istr("query_text");
    let query_index = istr("query_index");
    let depends = istr("DEPENDS_ON");
    let supports = istr("SUPPORTS");
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    let root_labels = || LabelSet::from_iter([doc.clone(), root.clone()]);
    let graph_root = mutator
        .create_node(
            root_labels(),
            props([(body.clone(), Value::String(istr("graph root")))]),
        )
        .expect("graph root inserts");
    let memory_root = mutator
        .create_node(
            root_labels(),
            props([(body.clone(), Value::String(istr("memory root")))]),
        )
        .expect("memory root inserts");
    let graph_strong = mutator
        .create_node(
            LabelSet::single(doc.clone()),
            props([(body.clone(), Value::String(istr("graph graph memory")))]),
        )
        .expect("graph strong inserts");
    let graph_weak = mutator
        .create_node(
            LabelSet::single(doc.clone()),
            props([(body.clone(), Value::String(istr("graph retrieval")))]),
        )
        .expect("graph weak inserts");
    let memory_strong = mutator
        .create_node(
            LabelSet::single(doc.clone()),
            props([(body.clone(), Value::String(istr("memory memory notes")))]),
        )
        .expect("memory strong inserts");
    let no_match = mutator
        .create_node(
            LabelSet::single(doc),
            props([(body, Value::String(istr("vector only")))]),
        )
        .expect("non-match inserts");
    for (index, text, root_node) in [(0, "graph", graph_root), (1, "memory", memory_root)] {
        let anchor = mutator
            .create_node(
                LabelSet::single(query_anchor.clone()),
                props([
                    (query_index.clone(), Value::Int(index)),
                    (query_text.clone(), Value::String(istr(text))),
                ]),
            )
            .expect("query anchor inserts");
        mutator
            .create_edge(depends.clone(), anchor, root_node, PropertyMap::new())
            .expect("query dependency edge inserts");
    }
    for target in [graph_strong, graph_weak, memory_strong, no_match] {
        mutator
            .create_edge(supports.clone(), graph_root, target, PropertyMap::new())
            .expect("graph support edge inserts");
    }
    for target in [graph_strong, memory_strong, no_match] {
        mutator
            .create_edge(supports.clone(), memory_root, target, PropertyMap::new())
            .expect("memory support edge inserts");
    }
    txn.commit().expect("seed graph commits");
    (graph_strong, graph_weak, memory_strong)
}
