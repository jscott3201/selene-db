//! Procedure-yielded graph-reference predicate regressions.

use selene_core::{GraphId, LabelSet, NodeId, PropertyMap, Value, db_string};
use selene_gql::{
    GqlType, ProcedureHandle, ProcedureOutputColumn, ProcedureResult, Session, StatementOutput,
};
use selene_graph::SharedGraph;
use selene_testing::MockProcedureRegistry;

fn dbs(value: &str) -> selene_core::DbString {
    db_string(value).expect("test fixture strings fit DB string cap")
}

fn graph_with_fact_and_note() -> (SharedGraph, NodeId, NodeId) {
    let graph = SharedGraph::new(GraphId::new(440_701));
    let mut txn = graph.begin_write();
    let (fact, note) = {
        let mut mutator = txn.mutator();
        let fact = mutator
            .create_node(LabelSet::single(dbs("Fact")), PropertyMap::new())
            .expect("fact node inserts");
        let note = mutator
            .create_node(LabelSet::single(dbs("Note")), PropertyMap::new())
            .expect("note node inserts");
        (fact, note)
    };
    txn.commit().expect("graph commit succeeds");
    (graph, fact, note)
}

fn node_registry(fact: NodeId, note: NodeId) -> MockProcedureRegistry {
    MockProcedureRegistry::new()
        .with_procedure(
            vec![dbs("pkg"), dbs("nodes")],
            Vec::new(),
            vec![ProcedureOutputColumn::new(dbs("node"), GqlType::NodeRef)],
        )
        .with_result(
            ProcedureHandle::new(1),
            ProcedureResult {
                rows: vec![vec![Value::NodeRef(note)], vec![Value::NodeRef(fact)]],
            },
        )
}

fn rows(output: StatementOutput) -> Vec<Vec<Value>> {
    match output {
        StatementOutput::Rows(table) => table
            .rows()
            .iter()
            .map(|row| row.values().to_vec())
            .collect(),
        other => panic!("expected rows, got {other:?}"),
    }
}

#[test]
fn procedure_yielded_node_ref_can_feed_label_predicate() {
    let (graph, fact, note) = graph_with_fact_and_note();
    let registry = node_registry(fact, note);
    let mut session = Session::new(&graph);

    let output = session
        .execute_source(
            "CALL pkg.nodes() YIELD node WHERE node IS LABELED :Fact RETURN node LIMIT 1",
            &registry,
        )
        .expect("procedure-yielded node label predicate executes");

    assert_eq!(rows(output), vec![vec![Value::NodeRef(fact)]]);
}
