//! End-to-end coverage for reachability candidate production.

use selene_core::{DbString, GraphId, LabelSet, NodeId, PropertyMap, Value};
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

struct ReachabilitySeed {
    root: NodeId,
    child: NodeId,
    grandchild: NodeId,
    sibling: NodeId,
    wrong_label: NodeId,
}

fn seed_reachability_graph(graph: &SharedGraph) -> ReachabilitySeed {
    let node_label = db_string("ReachNode");
    let reach = db_string("REACHES");
    let other = db_string("OTHER");
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    let root = mutator
        .create_node(LabelSet::single(node_label.clone()), PropertyMap::new())
        .unwrap();
    let child = mutator
        .create_node(LabelSet::single(node_label.clone()), PropertyMap::new())
        .unwrap();
    let grandchild = mutator
        .create_node(LabelSet::single(node_label.clone()), PropertyMap::new())
        .unwrap();
    let sibling = mutator
        .create_node(LabelSet::single(node_label.clone()), PropertyMap::new())
        .unwrap();
    let wrong_label = mutator
        .create_node(LabelSet::single(node_label), PropertyMap::new())
        .unwrap();
    mutator
        .create_edge(reach.clone(), root, child, PropertyMap::new())
        .unwrap();
    mutator
        .create_edge(reach.clone(), child, grandchild, PropertyMap::new())
        .unwrap();
    mutator
        .create_edge(reach.clone(), root, sibling, PropertyMap::new())
        .unwrap();
    mutator
        .create_edge(other, root, wrong_label, PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();
    ReachabilitySeed {
        root,
        child,
        grandchild,
        sibling,
        wrong_label,
    }
}

#[test]
fn reachable_nodes_returns_transitive_candidates_with_depths() {
    let graph = graph(440_001);
    let registry = BuiltinProcedureRegistry::new();
    let seed = seed_reachability_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("roots"),
        Value::List(vec![Value::NodeRef(seed.root)]),
    );

    let table = execute_rows(
        &mut session,
        "CALL selene.reachable_nodes($roots, 'REACHES', 10) \
         YIELD node_id, depth",
        &registry,
    );

    assert_eq!(
        node_column(&table, "node_id"),
        vec![seed.root, seed.child, seed.sibling, seed.grandchild]
    );
    assert_eq!(uint_column(&table, "depth"), vec![0, 1, 1, 2]);
    assert!(!node_column(&table, "node_id").contains(&seed.wrong_label));
}

#[test]
fn reachable_nodes_honors_incoming_depth_and_k() {
    let graph = graph(440_002);
    let registry = BuiltinProcedureRegistry::new();
    let seed = seed_reachability_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("roots"),
        Value::List(vec![Value::NodeRef(seed.grandchild)]),
    );

    let incoming = execute_rows(
        &mut session,
        "CALL selene.reachable_nodes($roots, 'REACHES', 10, 1, 'incoming') \
         YIELD node_id, depth",
        &registry,
    );

    assert_eq!(
        node_column(&incoming, "node_id"),
        vec![seed.grandchild, seed.child]
    );
    assert_eq!(uint_column(&incoming, "depth"), vec![0, 1]);

    let capped = execute_rows(
        &mut session,
        "CALL selene.reachable_nodes($roots, 'REACHES', 1, NULL, 'incoming') \
         YIELD node_id, depth",
        &registry,
    );
    assert_eq!(node_column(&capped, "node_id"), vec![seed.grandchild]);
}

#[test]
fn reachable_nodes_rejects_invalid_direction() {
    let graph = graph(440_003);
    let registry = BuiltinProcedureRegistry::new();
    let seed = seed_reachability_graph(&graph);
    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("roots"),
        Value::List(vec![Value::NodeRef(seed.root)]),
    );

    let err = session
        .execute_source(
            "CALL selene.reachable_nodes($roots, 'REACHES', 10, NULL, 'sideways')",
            &registry,
        )
        .expect_err("invalid direction must error");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("unknown reachability direction")
    ));
}
