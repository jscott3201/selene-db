//! BRIEF-131a catalog DDL idempotency tests.

mod exec_common;

use selene_core::{Change, GraphId, LabelSet, SchemaChange};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, ExecutionPlan,
    ExecutorError, GqlStatus, TxContext, execute_pipeline,
};
use selene_graph::{GraphError, GraphTypeDef, NodeTypeDef, SharedGraph};

use exec_common::{istr, planned};

fn seed_table() -> BindingTable {
    BindingTable::new(
        BindingTableSchema {
            columns: Vec::new(),
        },
        vec![Binding::empty()],
    )
}

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: istr("catalog.idempotency.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

fn person_graph(id: u64) -> SharedGraph {
    let person = istr("Person");
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: istr("catalog.idempotency.person.graph"),
            node_types: vec![NodeTypeDef {
                name: person,
                key_labels: LabelSet::single(person),
                properties: Vec::new(),
            }],
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

fn run_write(
    graph: &SharedGraph,
    plan: &ExecutionPlan,
) -> Result<
    (
        BindingTable,
        Result<selene_graph::CommitOutcome, GraphError>,
    ),
    ExecutorError,
> {
    let snapshot = graph.read();
    let mut txn = graph.begin_write();
    let result = {
        let mut ctx = TxContext::write(
            snapshot,
            &plan.impl_defined_caps,
            &EmptyProcedureRegistry,
            &mut txn,
            graph.index_providers(),
        );
        execute_pipeline(&plan.pipeline, seed_table(), &mut ctx)
    };
    match result {
        Ok(table) => Ok((table, txn.commit())),
        Err(error) => {
            txn.rollback();
            Err(error)
        }
    }
}

#[test]
fn create_node_type_if_not_exists_creates_then_noops() {
    let graph = empty_closed_graph(131_001);
    let plan = planned("CREATE NODE TYPE IF NOT EXISTS :Person ()");

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog creates");
    let outcome = outcome.expect("first commit succeeds");
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::NodeTypeAdded { label, .. },
            ..
        }] if label.as_str() == "Person"
    ));

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog noops");
    let outcome = outcome.expect("second commit succeeds");
    assert!(outcome.changes.is_empty());
    assert_eq!(graph.graph_type().unwrap().node_types.len(), 1);
}

#[test]
fn create_node_type_without_if_not_exists_duplicate_returns_duplicate_object() {
    let graph = person_graph(131_002);
    let plan = planned("CREATE NODE TYPE :Person ()");

    let err = run_write(&graph, &plan).expect_err("duplicate type fails");

    assert_eq!(err.gqlstatus(), GqlStatus::DUPLICATE_OBJECT);
}

#[test]
fn create_edge_type_if_not_exists_creates_then_noops() {
    let graph = person_graph(131_003);
    let plan = planned("CREATE EDGE TYPE IF NOT EXISTS :KNOWS (FROM :Person TO :Person)");

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog creates");
    let outcome = outcome.expect("first commit succeeds");
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::EdgeTypeAdded { label, .. },
            ..
        }] if label.as_str() == "KNOWS"
    ));

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog noops");
    let outcome = outcome.expect("second commit succeeds");
    assert!(outcome.changes.is_empty());
    assert_eq!(graph.graph_type().unwrap().edge_types.len(), 1);
}

#[test]
fn create_edge_type_without_if_not_exists_duplicate_returns_duplicate_object() {
    let graph = person_graph(131_004);
    let create = planned("CREATE EDGE TYPE :KNOWS (FROM :Person TO :Person)");
    run_write(&graph, &create)
        .expect("catalog creates")
        .1
        .expect("first commit succeeds");

    let err = run_write(&graph, &create).expect_err("duplicate edge type fails");

    assert_eq!(err.gqlstatus(), GqlStatus::DUPLICATE_OBJECT);
}

#[test]
fn drop_node_type_if_exists_drops_then_noops() {
    let graph = person_graph(131_005);
    let plan = planned("DROP NODE TYPE IF EXISTS :Person");

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog drops");
    let outcome = outcome.expect("first commit succeeds");
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::NodeTypeDropped { name, .. },
            ..
        }] if name.as_str() == "Person"
    ));
    assert!(graph.graph_type().unwrap().node_types.is_empty());

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog noops");
    let outcome = outcome.expect("second commit succeeds");
    assert!(outcome.changes.is_empty());
}

#[test]
fn drop_edge_type_if_exists_drops_then_noops() {
    let graph = person_graph(131_006);
    let create = planned("CREATE EDGE TYPE :KNOWS (FROM :Person TO :Person)");
    run_write(&graph, &create)
        .expect("catalog creates")
        .1
        .expect("create commit succeeds");
    let plan = planned("DROP EDGE TYPE IF EXISTS :KNOWS");

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog drops");
    let outcome = outcome.expect("drop commit succeeds");
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::EdgeTypeDropped { name, .. },
            ..
        }] if name.as_str() == "KNOWS"
    ));
    assert!(graph.graph_type().unwrap().edge_types.is_empty());

    let (_table, outcome) = run_write(&graph, &plan).expect("catalog noops");
    let outcome = outcome.expect("second commit succeeds");
    assert!(outcome.changes.is_empty());
}

#[test]
fn drop_nonexistent_edge_type_returns_data_exception() {
    let graph = person_graph(131_007);
    let plan = planned("DROP EDGE TYPE :Missing");

    let err = run_write(&graph, &plan).expect_err("drop missing fails");

    assert!(matches!(
        err,
        ExecutorError::GraphTypeViolation { message, .. }
            if message.contains("edge type Missing does not exist")
    ));
}

#[test]
fn drop_type_if_exists_absent_noops() {
    let graph = empty_closed_graph(131_008);
    for source in [
        "DROP NODE TYPE IF EXISTS :Missing",
        "DROP EDGE TYPE IF EXISTS :Missing",
    ] {
        let plan = planned(source);

        let (_table, outcome) = run_write(&graph, &plan).expect("catalog noops");
        let outcome = outcome.expect("commit succeeds");
        assert!(outcome.changes.is_empty());
    }
}
