//! End-to-end edge-property index execution tests through `Session`.

use selene_core::{DbString, EdgeId, GraphId, LabelSet, PropertyMap, Value};
use selene_gql::{BindingTable, EmptyProcedureRegistry, Session, StatementOutput};
use selene_graph::{SharedGraph, TypedIndexKind};

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn props<const N: usize>(pairs: [(DbString, Value); N]) -> PropertyMap {
    PropertyMap::from_pairs(pairs).expect("test properties fit caps")
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("write returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn edge_ids(table: &BindingTable) -> Vec<u64> {
    table
        .rows()
        .iter()
        .filter_map(|row| match row.values().first() {
            Some(Value::EdgeRef(id)) => Some(id.get()),
            _ => None,
        })
        .collect()
}

fn explain_dump(session: &mut Session<'_>, source: &str) -> String {
    let table = rows(
        session
            .execute_source(&format!("EXPLAIN {source}"), &EmptyProcedureRegistry)
            .expect("EXPLAIN executes"),
    );
    match table.rows().first().and_then(|r| r.values().first()) {
        Some(Value::String(dump)) => dump.as_str().to_owned(),
        other => panic!("expected EXPLAIN dump string, got {other:?}"),
    }
}

fn build_edge_graph() -> (SharedGraph, Vec<EdgeId>) {
    let graph = SharedGraph::new(GraphId::new(908));
    let block = db_string("Block");
    let connected = db_string("CONNECTED_TO");
    let from_port = db_string("from_port");
    let pin = db_string("pin");
    let mut edges = Vec::new();
    {
        let mut txn = graph.begin_write();
        {
            let mut m = txn.mutator();
            let a = m
                .create_node(LabelSet::single(block.clone()), PropertyMap::new())
                .unwrap();
            let b = m
                .create_node(LabelSet::single(block.clone()), PropertyMap::new())
                .unwrap();
            let c = m
                .create_node(LabelSet::single(block), PropertyMap::new())
                .unwrap();
            edges.push(
                m.create_edge(
                    connected.clone(),
                    a,
                    b,
                    props([
                        (from_port.clone(), Value::String(db_string("out_0"))),
                        (pin.clone(), Value::Int(8)),
                    ]),
                )
                .unwrap(),
            );
            edges.push(
                m.create_edge(
                    connected.clone(),
                    b,
                    c,
                    props([
                        (from_port.clone(), Value::String(db_string("out_1"))),
                        (pin.clone(), Value::Int(13)),
                    ]),
                )
                .unwrap(),
            );
            edges.push(
                m.create_edge(
                    connected.clone(),
                    c,
                    a,
                    props([
                        (from_port.clone(), Value::String(db_string("aux"))),
                        (pin.clone(), Value::Int(21)),
                    ]),
                )
                .unwrap(),
            );
        }
        txn.commit().unwrap();
    }
    graph
        .create_edge_property_index(connected.clone(), from_port, TypedIndexKind::String)
        .unwrap();
    graph
        .create_edge_property_index(connected, pin, TypedIndexKind::I64)
        .unwrap();
    (graph, edges)
}

#[test]
fn edge_typed_index_returns_same_rows_as_linear() {
    let (graph, edges) = build_edge_graph();
    let source = "MATCH ()-[e:CONNECTED_TO]->() WHERE e.from_port = 'out_1' RETURN e";

    let mut indexed = Session::new(&graph);
    let indexed_rows = edge_ids(&rows(
        indexed
            .execute_source(source, &EmptyProcedureRegistry)
            .unwrap(),
    ));

    let mut linear = Session::new(&graph).without_index_selection();
    let linear_rows = edge_ids(&rows(
        linear
            .execute_source(source, &EmptyProcedureRegistry)
            .unwrap(),
    ));

    assert_eq!(indexed_rows, linear_rows);
    assert_eq!(indexed_rows, vec![edges[1].get()]);

    let dump = explain_dump(&mut indexed, source);
    assert!(
        dump.contains("TypedIndexRange"),
        "edge equality should render TypedIndexRange; got:\n{dump}"
    );
}

#[test]
fn edge_range_index_returns_same_rows_as_linear() {
    let (graph, edges) = build_edge_graph();
    let source = "MATCH ()-[e:CONNECTED_TO]->() WHERE e.pin >= 10 AND e.pin < 20 RETURN e";

    let mut indexed = Session::new(&graph);
    let indexed_rows = edge_ids(&rows(
        indexed
            .execute_source(source, &EmptyProcedureRegistry)
            .unwrap(),
    ));

    let mut linear = Session::new(&graph).without_index_selection();
    let linear_rows = edge_ids(&rows(
        linear
            .execute_source(source, &EmptyProcedureRegistry)
            .unwrap(),
    ));

    assert_eq!(indexed_rows, linear_rows);
    assert_eq!(indexed_rows, vec![edges[1].get()]);

    let dump = explain_dump(&mut indexed, source);
    assert!(
        dump.contains("TypedIndexRange"),
        "edge range should render TypedIndexRange; got:\n{dump}"
    );
}

#[test]
fn edge_in_list_index_returns_same_rows_as_linear() {
    let (graph, edges) = build_edge_graph();
    let source = "MATCH ()-[e:CONNECTED_TO]->() WHERE e.from_port IN ['out_0', 'aux'] RETURN e";

    let mut indexed = Session::new(&graph);
    let indexed_rows = edge_ids(&rows(
        indexed
            .execute_source(source, &EmptyProcedureRegistry)
            .unwrap(),
    ));

    let mut linear = Session::new(&graph).without_index_selection();
    let linear_rows = edge_ids(&rows(
        linear
            .execute_source(source, &EmptyProcedureRegistry)
            .unwrap(),
    ));

    assert_eq!(indexed_rows, linear_rows);
    assert_eq!(indexed_rows, vec![edges[0].get(), edges[2].get()]);

    let dump = explain_dump(&mut indexed, source);
    assert!(
        dump.contains("BitmapUnion"),
        "edge IN list should render BitmapUnion; got:\n{dump}"
    );
}

#[test]
fn indexed_undirected_edge_expand_preserves_linear_duplicates() {
    let (graph, edges) = build_edge_graph();
    let source = "MATCH ()-[e:CONNECTED_TO]-() WHERE e.from_port = 'out_1' RETURN e";

    let mut indexed = Session::new(&graph);
    let indexed_rows = edge_ids(&rows(
        indexed
            .execute_source(source, &EmptyProcedureRegistry)
            .unwrap(),
    ));

    let mut linear = Session::new(&graph).without_index_selection();
    let linear_rows = edge_ids(&rows(
        linear
            .execute_source(source, &EmptyProcedureRegistry)
            .unwrap(),
    ));

    assert_eq!(indexed_rows, linear_rows);
    assert_eq!(indexed_rows, vec![edges[1].get(), edges[1].get()]);
}
