//! Filtered BM25 `selene.text_search_nodes` coverage.

use selene_core::{DbString, GraphId, LabelSet, NodeId, PropertyMap, Value};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ProcedureRegistry, Session, StatementOutput,
};
use selene_graph::{SharedGraph, TypedIndexKind};

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn props(body: &DbString, text: &str, namespace: &DbString, scope: &str) -> PropertyMap {
    PropertyMap::from_pairs([
        (body.clone(), Value::String(db_string(text))),
        (namespace.clone(), Value::String(db_string(scope))),
    ])
    .expect("test property map is valid")
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
        .zip(float_column(table, "score"))
        .collect()
}

fn seed_fixture(graph: &SharedGraph, with_text_index: bool) -> Vec<NodeId> {
    let doc = db_string("TextDoc");
    let body = db_string("body");
    let namespace = db_string("namespace");
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    let specs = [
        ("visible", "graph graph memory retrieval"),
        ("hidden", "graph retrieval retrieval"),
        ("visible", "retrieval graph memory"),
        ("hidden", "graph graph"),
        ("visible", "unrelated archive"),
    ];
    let mut visible = Vec::new();
    for (scope, text) in specs {
        let node = mutator
            .create_node(
                LabelSet::single(doc.clone()),
                props(&body, text, &namespace, scope),
            )
            .expect("text node inserts");
        if scope == "visible" {
            visible.push(node);
        }
    }
    mutator
        .create_property_index(doc.clone(), namespace.clone(), TypedIndexKind::String)
        .expect("namespace index creates");
    if with_text_index {
        mutator
            .create_text_index(doc, body)
            .expect("text index creates");
    }
    txn.commit().expect("seed commits");
    visible
}

fn assert_filtered_text_search_matches_full_rust_filter(graph_id: u64, with_text_index: bool) {
    let graph = graph(graph_id);
    let visible = seed_fixture(&graph, with_text_index);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
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
        "CALL selene.text_search_nodes('TextDoc', 'body', 'graph retrieval', 10) \
         YIELD node_id, score",
        &registry,
    );
    let expected = hit_pairs(&unfiltered)
        .into_iter()
        .filter(|(node, _)| visible.contains(node))
        .take(3)
        .collect::<Vec<_>>();

    let filtered = execute_rows(
        &mut session,
        "CALL selene.text_search_nodes('TextDoc', 'body', 'graph retrieval', 3, 'namespace', $visible) \
         YIELD node_id, score",
        &registry,
    );
    assert_eq!(hit_pairs(&filtered), expected);

    let empty = execute_rows(
        &mut session,
        "CALL selene.text_search_nodes('TextDoc', 'body', 'graph retrieval', 3, 'namespace', $empty) \
         YIELD node_id, score",
        &registry,
    );
    assert_eq!(empty.row_count(), 0);

    let superset = execute_rows(
        &mut session,
        "CALL selene.text_search_nodes('TextDoc', 'body', 'graph retrieval', 10, 'namespace', $all_scopes) \
         YIELD node_id, score",
        &registry,
    );
    assert_eq!(hit_pairs(&superset), hit_pairs(&unfiltered));

    let null_filter = execute_rows(
        &mut session,
        "CALL selene.text_search_nodes('TextDoc', 'body', 'graph retrieval', 10, NULL, NULL) \
         YIELD node_id, score",
        &registry,
    );
    assert_eq!(hit_pairs(&null_filter), hit_pairs(&unfiltered));
}

#[test]
fn text_search_filter_matches_exact_scan_full_ranking() {
    assert_filtered_text_search_matches_full_rust_filter(431_701, false);
}

#[test]
fn text_search_filter_matches_maintained_text_index_full_ranking() {
    assert_filtered_text_search_matches_full_rust_filter(431_702, true);
}
