//! BRIEF-151 DROP TYPE RESTRICT/CASCADE executor differential tests.
//!
//! Exercises the Seam-B fix (deletion-reclamation audit Item 3) through the GQL
//! executor pipeline: RESTRICT (default) rejects a drop with surviving instances
//! or an inbound edge-type dependency with `G2000` and drops nothing; CASCADE
//! truncates instances first then drops the type, atomically in one transaction.

mod exec_common;

use selene_core::{Change, GraphId, LabelSet, PropertyMap, SchemaChange};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, ExecutionPlan,
    ExecutorError, TxContext, execute_pipeline,
};
use selene_graph::{
    CommitOutcome, EdgeEndpointDef, EdgeTypeDef, GraphError, GraphTypeDef, NodeTypeDef,
    SharedGraph, ValidationMode,
};

use exec_common::{istr, planned};

fn seed_table() -> BindingTable {
    BindingTable::new(
        BindingTableSchema {
            columns: Vec::new(),
        },
        vec![Binding::empty()],
    )
}

fn run_write(
    graph: &SharedGraph,
    plan: &ExecutionPlan,
) -> Result<(BindingTable, Result<CommitOutcome, GraphError>), ExecutorError> {
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

fn closed_graph_with_type(id: u64, graph_type: GraphTypeDef) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap()
}

fn person_graph(id: u64) -> SharedGraph {
    let person = istr("Person");
    closed_graph_with_type(
        id,
        GraphTypeDef {
            name: istr("catalog.person.graph"),
            node_types: vec![NodeTypeDef {
                name: person,
                key_labels: LabelSet::single(person),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            }],
            edge_types: Vec::new(),
        },
    )
}

/// KNOWS edge type whose endpoints index-reference Person (`NodeType(0)`).
fn person_self_knows_graph(id: u64) -> SharedGraph {
    let person = istr("Person");
    let knows = istr("KNOWS");
    closed_graph_with_type(
        id,
        GraphTypeDef {
            name: istr("catalog.person.knows.graph"),
            node_types: vec![NodeTypeDef {
                name: person,
                key_labels: LabelSet::single(person),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            }],
            edge_types: vec![EdgeTypeDef {
                name: knows,
                label: knows,
                source_node_type: EdgeEndpointDef::NodeType(0),
                target_node_type: EdgeEndpointDef::NodeType(0),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            }],
        },
    )
}

/// KNOWS edge type with `Any` endpoints — no node-type index dependency, so
/// dropping Person does not trip the reindexing guard.
fn person_any_knows_graph(id: u64) -> SharedGraph {
    let person = istr("Person");
    let knows = istr("KNOWS");
    closed_graph_with_type(
        id,
        GraphTypeDef {
            name: istr("catalog.person.any.knows.graph"),
            node_types: vec![NodeTypeDef {
                name: person,
                key_labels: LabelSet::single(person),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            }],
            edge_types: vec![EdgeTypeDef {
                name: knows,
                label: knows,
                source_node_type: EdgeEndpointDef::Any,
                target_node_type: EdgeEndpointDef::Any,
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            }],
        },
    )
}

#[test]
fn drop_node_type_restrict_default_rejects_early_with_g2000() {
    // Seam-B fix: RESTRICT is the default, so dropping a node type with
    // surviving instances rejects EARLY (at the drop op, not late at commit)
    // with G2000 GraphTypeViolation and drops NOTHING — the type and its
    // instances both survive.
    let graph = person_graph(3711);
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(istr("Person")), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
    }
    let plan = planned("DROP NODE TYPE :Person");

    let err = run_write(&graph, &plan).expect_err("RESTRICT default rejects the drop op");
    assert!(matches!(
        err,
        ExecutorError::GraphTypeViolation { message, .. }
            if message.contains("instance(s) still exist") && message.contains("CASCADE")
    ));
    assert_eq!(graph.graph_type().unwrap().node_types.len(), 1);
    assert_eq!(graph.read().node_count(), 1);
}

#[test]
fn drop_node_type_explicit_restrict_rejects_with_g2000() {
    let graph = person_graph(37111);
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(istr("Person")), PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
    }
    let plan = planned("DROP NODE TYPE :Person RESTRICT");

    let err = run_write(&graph, &plan).expect_err("explicit RESTRICT rejects");
    assert!(matches!(err, ExecutorError::GraphTypeViolation { .. }));
    assert_eq!(graph.read().node_count(), 1);
}

#[test]
fn drop_node_type_cascade_truncates_then_drops_observably_equal_to_manual() {
    // CASCADE removes instances + incident edges AND drops the type, in one
    // committed transaction. The committed changeset is exactly the truncate
    // change followed by the schema drop, observably equal to
    // `TRUNCATE NODE TYPE :Person; DROP NODE TYPE :Person RESTRICT`.
    let graph = person_any_knows_graph(3713);
    let person = istr("Person");
    let knows = istr("KNOWS");
    {
        let mut txn = graph.begin_write();
        let a = txn
            .mutator()
            .create_node(LabelSet::single(person), PropertyMap::new())
            .unwrap();
        let b = txn
            .mutator()
            .create_node(LabelSet::single(person), PropertyMap::new())
            .unwrap();
        txn.mutator()
            .create_edge(knows, a, b, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
    }
    assert_eq!(graph.read().node_count(), 2);
    assert_eq!(graph.read().edge_count(), 1);

    let plan = planned("DROP NODE TYPE :Person CASCADE");
    let (_, outcome) = run_write(&graph, &plan).expect("CASCADE executes");
    let outcome = outcome.expect("CASCADE commits");

    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::NodesOfTypeTruncated { label },
            Change::SchemaChanged {
                change: SchemaChange::NodeTypeDropped { name, .. },
                ..
            },
        ] if *label == person && *name == person
    ));
    // No orphan instances, no dangling edges, type gone.
    assert_eq!(graph.read().node_count(), 0);
    assert_eq!(graph.read().edge_count(), 0);
    assert!(
        graph
            .graph_type()
            .unwrap()
            .node_types
            .iter()
            .all(|node_type| node_type.name != person)
    );
}

#[test]
fn drop_node_type_restrict_rejects_when_edge_type_references_it() {
    // Type-dependency RESTRICT: dropping a node type still index-referenced by
    // an edge type's endpoint would leave a dangling endpoint, so RESTRICT
    // rejects with a clear G2000 "still references it" message (no instances
    // needed).
    let graph = person_self_knows_graph(3715);
    let plan = planned("DROP NODE TYPE :Person");

    let err = run_write(&graph, &plan).expect_err("dependency rejects the drop");
    assert!(matches!(
        err,
        ExecutorError::GraphTypeViolation { message, .. }
            if message.contains("still references it")
    ));
    assert_eq!(graph.graph_type().unwrap().node_types.len(), 1);
    assert_eq!(graph.graph_type().unwrap().edge_types.len(), 1);
}

#[test]
fn drop_edge_type_restrict_rejects_with_surviving_edges_g2000() {
    let graph = person_self_knows_graph(3716);
    let person = istr("Person");
    let knows = istr("KNOWS");
    {
        let mut txn = graph.begin_write();
        let a = txn
            .mutator()
            .create_node(LabelSet::single(person), PropertyMap::new())
            .unwrap();
        let b = txn
            .mutator()
            .create_node(LabelSet::single(person), PropertyMap::new())
            .unwrap();
        txn.mutator()
            .create_edge(knows, a, b, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
    }
    let plan = planned("DROP EDGE TYPE :KNOWS");

    let err = run_write(&graph, &plan).expect_err("RESTRICT rejects edge drop");
    assert!(matches!(
        err,
        ExecutorError::GraphTypeViolation { message, .. }
            if message.contains("instance(s) still exist")
    ));
    assert_eq!(graph.graph_type().unwrap().edge_types.len(), 1);
    assert_eq!(graph.read().edge_count(), 1);
}

#[test]
fn drop_edge_type_cascade_removes_edges_then_drops_type() {
    let graph = person_self_knows_graph(37161);
    let person = istr("Person");
    let knows = istr("KNOWS");
    {
        let mut txn = graph.begin_write();
        let a = txn
            .mutator()
            .create_node(LabelSet::single(person), PropertyMap::new())
            .unwrap();
        let b = txn
            .mutator()
            .create_node(LabelSet::single(person), PropertyMap::new())
            .unwrap();
        txn.mutator()
            .create_edge(knows, a, b, PropertyMap::new())
            .unwrap();
        txn.commit().unwrap();
    }
    let plan = planned("DROP EDGE TYPE :KNOWS CASCADE");

    let (_, outcome) = run_write(&graph, &plan).expect("CASCADE executes");
    let outcome = outcome.expect("CASCADE commits");
    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::EdgesOfTypeTruncated { label },
            Change::SchemaChanged {
                change: SchemaChange::EdgeTypeDropped { name, .. },
                ..
            },
        ] if *label == knows && *name == knows
    ));
    assert_eq!(graph.read().edge_count(), 0);
    // The KNOWS endpoint nodes survive — only the edges of the type were removed.
    assert_eq!(graph.read().node_count(), 2);
    assert!(graph.graph_type().unwrap().edge_types.is_empty());
}

#[test]
fn drop_node_type_restrict_on_empty_type_drops_cleanly() {
    // Zero instances + RESTRICT (default) => drops exactly as before this brief.
    let graph = person_graph(37162);
    let plan = planned("DROP NODE TYPE :Person");

    let (_, outcome) = run_write(&graph, &plan).expect("empty-type drop executes");
    let outcome = outcome.expect("empty-type drop commits");
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::NodeTypeDropped { .. },
            ..
        }]
    ));
    assert!(graph.graph_type().unwrap().node_types.is_empty());
}

#[test]
fn drop_node_type_cascade_on_empty_type_drops_cleanly() {
    // Empty type + CASCADE => truncate is a no-op, then drop; observationally
    // identical to RESTRICT on an empty type (single NodeTypeDropped change).
    let graph = person_graph(37163);
    let plan = planned("DROP NODE TYPE :Person CASCADE");

    let (_, outcome) = run_write(&graph, &plan).expect("empty-type cascade executes");
    let outcome = outcome.expect("empty-type cascade commits");
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::NodeTypeDropped { .. },
            ..
        }]
    ));
    assert!(graph.graph_type().unwrap().node_types.is_empty());
}
