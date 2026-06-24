//! Filtered `selene.vector_search_nodes_ann` coverage.

use selene_core::{DbString, GraphId, LabelSet, NodeId, PropertyMap, Value, VectorValue};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ProcedureRegistry, Session, StatementOutput,
};
use selene_graph::{SharedGraph, TypedIndexKind, VectorIndexKind};

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn vector(components: &[f32]) -> VectorValue {
    VectorValue::new(components.to_vec()).expect("test vector is valid")
}

fn props(embedding: &DbString, idx: usize, namespace: &DbString, scope: &str) -> PropertyMap {
    PropertyMap::from_pairs([
        (embedding.clone(), Value::Vector(vector(&[idx as f32, 0.0]))),
        (namespace.clone(), Value::String(db_string(scope))),
    ])
    .expect("test property map is valid")
}

fn edge_props(property: &DbString, value: &str) -> PropertyMap {
    PropertyMap::from_pairs([(property.clone(), Value::String(db_string(value)))])
        .expect("test edge property map is valid")
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

fn float_column(table: &BindingTable, name: &str) -> Vec<f64> {
    let index = table
        .column_index(db_string(name))
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

fn hit_pairs(table: &BindingTable) -> Vec<(NodeId, f64)> {
    node_column(table, "node_id")
        .into_iter()
        .zip(float_column(table, "distance"))
        .collect()
}

fn seed_fixture(graph: &SharedGraph, index_kind: VectorIndexKind) -> Vec<NodeId> {
    let doc = db_string("VectorDoc");
    let embedding = db_string("embedding");
    let namespace = db_string("namespace");
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    let mut visible = Vec::new();
    for idx in 0..32 {
        let scope = if idx % 5 == 0 { "visible" } else { "hidden" };
        let node = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, idx, &namespace, scope),
            )
            .expect("vector node inserts");
        if scope == "visible" {
            visible.push(node);
        }
    }
    mutator
        .create_property_index(doc.clone(), namespace.clone(), TypedIndexKind::String)
        .expect("namespace index creates");
    mutator
        .create_vector_index(doc, embedding, index_kind, 2)
        .expect("vector index creates");
    txn.commit().expect("seed commits");
    visible
}

fn seed_edge_filter_fixture(
    graph: &SharedGraph,
    index_kind: VectorIndexKind,
) -> (Vec<NodeId>, Vec<NodeId>) {
    let doc = db_string("VectorDoc");
    let commit = db_string("Commit");
    let edge = db_string("AT_COMMIT");
    let embedding = db_string("embedding");
    let namespace = db_string("namespace");
    let commit_sha = db_string("commit_sha");
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    let commit_node = mutator
        .create_node(LabelSet::single(commit), PropertyMap::new())
        .expect("commit node inserts");
    let mut abc_nodes = Vec::new();
    let mut visible_abc_nodes = Vec::new();
    for idx in 0..32 {
        let scope = if idx % 5 == 0 { "visible" } else { "hidden" };
        let sha = if idx % 3 == 0 { "abc" } else { "def" };
        let node = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&embedding, idx, &namespace, scope),
            )
            .expect("vector node inserts");
        mutator
            .create_edge(
                edge.clone(),
                commit_node,
                node,
                edge_props(&commit_sha, sha),
            )
            .expect("commit edge inserts");
        if sha == "abc" {
            abc_nodes.push(node);
            if scope == "visible" {
                visible_abc_nodes.push(node);
            }
        }
    }
    mutator
        .create_property_index(doc.clone(), namespace.clone(), TypedIndexKind::String)
        .expect("namespace index creates");
    mutator
        .create_edge_property_index(edge, commit_sha, TypedIndexKind::String)
        .expect("commit edge index creates");
    mutator
        .create_vector_index(doc, embedding, index_kind, 2)
        .expect("vector index creates");
    txn.commit().expect("seed commits");
    (abc_nodes, visible_abc_nodes)
}

fn assert_filtered_ann_matches_full_rust_filter(graph_id: u64, index_kind: VectorIndexKind) {
    let graph = graph(graph_id);
    let visible = seed_fixture(&graph, index_kind);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("query"), Value::Vector(vector(&[0.2, 0.0])));
    session.bind_parameter(
        db_string("visible"),
        Value::List(vec![Value::String(db_string("visible"))]),
    );
    session.bind_parameter(
        db_string("all_scopes"),
        Value::List(vec![
            Value::String(db_string("visible")),
            Value::String(db_string("hidden")),
        ]),
    );
    session.bind_parameter(db_string("empty"), Value::List(Vec::new()));

    let unfiltered = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_ann('VectorDoc', 'embedding', $query, 32, 'squared_euclidean', 64) \
         YIELD node_id, distance",
        &registry,
    );
    let expected = hit_pairs(&unfiltered)
        .into_iter()
        .filter(|(node, _)| visible.contains(node))
        .take(3)
        .collect::<Vec<_>>();

    let filtered = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_ann('VectorDoc', 'embedding', $query, 3, 'squared_euclidean', 64, 'namespace', $visible) \
         YIELD node_id, distance",
        &registry,
    );
    assert_eq!(hit_pairs(&filtered), expected);

    let empty = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_ann('VectorDoc', 'embedding', $query, 3, 'squared_euclidean', 64, 'namespace', $empty) \
         YIELD node_id, distance",
        &registry,
    );
    assert_eq!(empty.row_count(), 0);

    let superset = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_ann('VectorDoc', 'embedding', $query, 32, 'squared_euclidean', 64, 'namespace', $all_scopes) \
         YIELD node_id, distance",
        &registry,
    );
    assert_eq!(hit_pairs(&superset), hit_pairs(&unfiltered));

    let null_filter = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_ann('VectorDoc', 'embedding', $query, 32, 'squared_euclidean', 64, NULL, NULL) \
         YIELD node_id, distance",
        &registry,
    );
    assert_eq!(hit_pairs(&null_filter), hit_pairs(&unfiltered));
}

#[test]
fn vector_ann_filter_matches_hnsw_full_ranking() {
    assert_filtered_ann_matches_full_rust_filter(330_701, VectorIndexKind::HnswSquaredEuclidean);
}

#[test]
fn vector_ann_filter_matches_ivf_full_ranking() {
    assert_filtered_ann_matches_full_rust_filter(330_702, VectorIndexKind::IvfSquaredEuclidean);
}

fn assert_edge_filtered_ann_matches_full_rust_filter(graph_id: u64, index_kind: VectorIndexKind) {
    let graph = graph(graph_id);
    let (abc_nodes, visible_abc_nodes) = seed_edge_filter_fixture(&graph, index_kind);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("query"), Value::Vector(vector(&[0.2, 0.0])));
    session.bind_parameter(
        db_string("abc"),
        Value::List(vec![Value::String(db_string("abc"))]),
    );
    session.bind_parameter(
        db_string("visible"),
        Value::List(vec![Value::String(db_string("visible"))]),
    );

    let unfiltered = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_ann('VectorDoc', 'embedding', $query, 32, 'squared_euclidean', 64) \
         YIELD node_id, distance",
        &registry,
    );
    let expected_edge = hit_pairs(&unfiltered)
        .into_iter()
        .filter(|(node, _)| abc_nodes.contains(node))
        .take(3)
        .collect::<Vec<_>>();
    let expected_composed = hit_pairs(&unfiltered)
        .into_iter()
        .filter(|(node, _)| visible_abc_nodes.contains(node))
        .take(3)
        .collect::<Vec<_>>();

    let edge_filtered = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_ann(
             'VectorDoc', 'embedding', $query, 3, 'squared_euclidean', 64,
             NULL, NULL,
             'AT_COMMIT', 'commit_sha', $abc, 'target'
         ) YIELD node_id, distance",
        &registry,
    );
    assert_eq!(hit_pairs(&edge_filtered), expected_edge);

    let composed = execute_rows(
        &mut session,
        "CALL selene.vector_search_nodes_ann(
             'VectorDoc', 'embedding', $query, 3, 'squared_euclidean', 64,
             'namespace', $visible,
             'AT_COMMIT', 'commit_sha', $abc, 'target'
         ) YIELD node_id, distance",
        &registry,
    );
    assert_eq!(hit_pairs(&composed), expected_composed);
}

#[test]
fn vector_ann_edge_filter_matches_hnsw_full_ranking() {
    assert_edge_filtered_ann_matches_full_rust_filter(
        330_703,
        VectorIndexKind::HnswSquaredEuclidean,
    );
}

#[test]
fn vector_ann_edge_filter_matches_ivf_full_ranking() {
    assert_edge_filtered_ann_matches_full_rust_filter(
        330_704,
        VectorIndexKind::IvfSquaredEuclidean,
    );
}
