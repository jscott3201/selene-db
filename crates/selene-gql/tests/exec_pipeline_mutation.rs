//! BRIEF-36 mutation pipeline executor tests.

mod exec_common;

use selene_core::{Change, EdgeId, GraphId, LabelSet, NodeId, PropertyMap, Value};
use selene_gql::{
    AnalyzedType, Binding, BindingTable, BindingTableColumn, BindingTableSchema, EdgeDirection,
    EmptyProcedureRegistry, ExecutionPlan, ExecutorError, GqlStatus, GqlType, MutationOp,
    PipelineOp, TxContext, analyze, execute_pattern, execute_pipeline, parse, plan,
};
use selene_graph::{CommitOutcome, SharedGraph};

use exec_common::{column_values, istr, props};

fn planned(source: &str) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
}

fn seed_table() -> BindingTable {
    BindingTable::new(
        BindingTableSchema { columns: vec![] },
        vec![Binding::empty()],
    )
}

fn run_write(
    graph: &SharedGraph,
    plan: &ExecutionPlan,
) -> Result<(BindingTable, CommitOutcome), ExecutorError> {
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
        let input = if let Some(pattern) = &plan.pattern_plan {
            execute_pattern(pattern, &ctx)?
        } else {
            seed_table()
        };
        execute_pipeline(&plan.pipeline, input, &mut ctx)
    };
    match result {
        Ok(table) => {
            let outcome = txn.commit().expect("write commits");
            Ok((table, outcome))
        }
        Err(error) => {
            txn.rollback();
            Err(error)
        }
    }
}

fn empty_graph() -> SharedGraph {
    SharedGraph::new(GraphId::new(3600))
}

fn graph_with_person(name: &str) -> SharedGraph {
    let graph = empty_graph();
    {
        let person = istr("Person");
        let name_key = istr("name");
        let age = istr("age");
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(person),
                props([(name_key, Value::String(istr(name))), (age, Value::Int(30))]),
            )
            .expect("person inserts");
        txn.commit().expect("fixture commits");
    }
    graph
}

fn graph_with_edge() -> SharedGraph {
    let graph = empty_graph();
    {
        let victim = istr("Victim");
        let other = istr("Other");
        let rel = istr("REL");
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        let a = mutator
            .create_node(LabelSet::single(victim), PropertyMap::new())
            .expect("node inserts");
        let b = mutator
            .create_node(LabelSet::single(other), PropertyMap::new())
            .expect("node inserts");
        mutator
            .create_edge(rel, a, b, PropertyMap::new())
            .expect("edge inserts");
        txn.commit().expect("fixture commits");
    }
    graph
}

fn first_node(table: &BindingTable, column: &str) -> NodeId {
    match column_values(table, column).first().expect("row exists") {
        Value::NodeRef(id) => *id,
        other => panic!("expected node ref, got {other:?}"),
    }
}

#[test]
fn insert_node_with_label_creates_node_in_graph() {
    let graph = empty_graph();
    let plan = planned("INSERT (n:Person) RETURN n");

    let (table, outcome) = run_write(&graph, &plan).expect("write executes");

    let node = first_node(&table, "n");
    let snapshot = graph.read();
    assert_eq!(snapshot.node_count(), 1);
    assert!(
        snapshot
            .node_labels(node)
            .unwrap()
            .contains(&istr("Person"))
    );
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::NodeCreated { .. }]
    ));
}

#[test]
fn insert_node_extends_row_with_new_node_id_at_planner_assigned_column() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (p:Person) INSERT (n:Person) RETURN p, n");

    let (table, _) = run_write(&graph, &plan).expect("write executes");

    assert_eq!(table.schema().columns[0].name.unwrap().as_str(), "p");
    assert_eq!(table.schema().columns[1].name.unwrap().as_str(), "n");
    assert!(matches!(table.rows()[0].get(0), Some(Value::NodeRef(_))));
    assert!(matches!(table.rows()[0].get(1), Some(Value::NodeRef(_))));
}

#[test]
fn insert_node_with_property_initializers_evaluates_per_row() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (p:Person) INSERT (n:Copy {name: p.name}) RETURN n.name AS name");

    let (table, _) = run_write(&graph, &plan).expect("write executes");

    assert_eq!(
        column_values(&table, "name"),
        [Value::String(istr("Alice"))]
    );
}

#[test]
fn insert_edge_between_two_matched_bindings_creates_edge() {
    let graph = empty_graph();
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(LabelSet::single(istr("A")), PropertyMap::new())
            .expect("node inserts");
        mutator
            .create_node(LabelSet::single(istr("B")), PropertyMap::new())
            .expect("node inserts");
        txn.commit().expect("fixture commits");
    }
    let plan = planned("MATCH (a:A), (b:B) INSERT (a)-[:REL]->(b) RETURN a, b");

    let (_, _) = run_write(&graph, &plan).expect("write executes");

    assert_eq!(graph.read().edge_count(), 1);
}

#[test]
fn undirected_insert_edge_is_rejected_at_runtime() {
    let graph = empty_graph();
    let mut plan = planned("INSERT (:A)-[:REL]->(:B) RETURN 1 AS ok");
    let edge = plan
        .pipeline
        .iter_mut()
        .find_map(|op| match op {
            PipelineOp::Mutation(MutationOp::InsertEdge { direction, .. }) => Some(direction),
            _ => None,
        })
        .expect("plan inserts an edge");
    *edge = EdgeDirection::Undirected;

    let err = run_write(&graph, &plan).expect_err("undirected INSERT edge rejects");

    assert!(matches!(
        err,
        ExecutorError::ImplementationDefined {
            detail: "INSERT undirected edge not implemented",
        }
    ));
    assert_eq!(graph.read().edge_count(), 0);
}

#[test]
fn chain_insert_with_anonymous_middle_node_links_correctly() {
    let graph = empty_graph();
    let plan = planned("INSERT (a:A)-[:R]->(:M)-[:S]->(b:B) RETURN a, b");

    let (table, _) = run_write(&graph, &plan).expect("write executes");

    let a = first_node(&table, "a");
    let b = first_node(&table, "b");
    let snapshot = graph.read();
    let middle = snapshot
        .outgoing_edges(a)
        .and_then(|edges| edges.iter().next())
        .map(|edge| edge.neighbor)
        .expect("a has outgoing edge");
    assert_ne!(middle, b);
    assert_eq!(
        snapshot
            .outgoing_edges(middle)
            .and_then(|edges| edges.iter().next())
            .map(|edge| edge.neighbor),
        Some(b)
    );
}

#[test]
fn multi_row_insert_preserves_input_row_order_in_new_id_column() {
    let fixture = exec_common::ExecFixture::build();
    let plan = planned("MATCH (p:Person) INSERT (n:Copy {name: p.name}) RETURN n");

    let (table, _) = run_write(&fixture.graph, &plan).expect("write executes");
    let ids = column_values(&table, "n")
        .into_iter()
        .map(|value| match value {
            Value::NodeRef(id) => id.get(),
            other => panic!("expected node ref, got {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(ids.len(), 3);
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn set_property_evaluates_per_row_against_input_row() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n:Person) SET n.age = n.age + 12 RETURN n.age AS age");

    let (table, _) = run_write(&graph, &plan).expect("write executes");

    assert_eq!(column_values(&table, "age"), [Value::Int(42)]);
}

#[test]
fn set_property_to_null_stores_explicit_null() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n:Person) SET n.age = NULL RETURN n");

    let (table, _) = run_write(&graph, &plan).expect("write executes");
    let id = first_node(&table, "n");

    assert_eq!(
        graph.read().node_properties(id).unwrap().get(&istr("age")),
        Some(&Value::Null)
    );
}

#[test]
fn set_label_on_existing_node_adds_label_idempotently() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n:Person) SET n :Person RETURN n");

    let (table, _) = run_write(&graph, &plan).expect("write executes");
    let id = first_node(&table, "n");

    assert!(
        graph
            .read()
            .node_labels(id)
            .unwrap()
            .contains(&istr("Person"))
    );
}

#[test]
fn remove_property_removes_when_present() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n:Person) REMOVE n.age RETURN n");

    let (table, outcome) = run_write(&graph, &plan).expect("write executes");
    let id = first_node(&table, "n");

    assert_eq!(
        graph.read().node_properties(id).unwrap().get(&istr("age")),
        None
    );
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::NodePropertyRemoved { id: changed, property }]
            if *changed == id && *property == istr("age")
    ));
}

#[test]
fn remove_nonexistent_property_is_idempotent() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n:Person) REMOVE n.missing RETURN n");

    let (table, outcome) = run_write(&graph, &plan).expect("write executes");
    let id = first_node(&table, "n");

    assert!(graph.read().is_node_alive(id));
    assert!(outcome.changes.is_empty());
}

#[test]
fn remove_label_removes_when_present() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n:Person) REMOVE n :Person RETURN n");

    let (table, outcome) = run_write(&graph, &plan).expect("write executes");
    let id = first_node(&table, "n");

    assert!(
        !graph
            .read()
            .node_labels(id)
            .unwrap()
            .contains(&istr("Person"))
    );
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::NodeLabelRemoved { id: changed, label }]
            if *changed == id && *label == istr("Person")
    ));
}

#[test]
fn remove_edge_property_removes_when_present() {
    let graph = graph_with_edge();
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .update_edge(
                EdgeId::new(1),
                selene_core::PropertyDiff::new([(istr("since"), Value::Int(2026))], []).unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    let plan = planned("MATCH ()-[r:REL]->() REMOVE r.since FINISH");

    let (_, outcome) = run_write(&graph, &plan).expect("write executes");

    assert!(
        graph
            .read()
            .edge_properties(EdgeId::new(1))
            .unwrap()
            .get(&istr("since"))
            .is_none()
    );
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::EdgePropertyRemoved { id, property }]
            if *id == EdgeId::new(1) && *property == istr("since")
    ));
}

#[test]
fn set_then_remove_emits_dedicated_changes_in_source_order() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n:Person) SET n.age = 31 REMOVE n.name FINISH");

    let (_, outcome) = run_write(&graph, &plan).expect("write executes");

    let snapshot = graph.read();
    let properties = snapshot.node_properties(NodeId::new(1)).unwrap();
    assert_eq!(properties.get(&istr("age")), Some(&Value::Int(31)));
    assert!(properties.get(&istr("name")).is_none());
    assert!(matches!(
        outcome.changes.as_slice(),
        [
            Change::NodeUpdated { id: updated, .. },
            Change::NodePropertyRemoved {
                id: removed,
                property
            }
        ] if *updated == NodeId::new(1)
            && *removed == NodeId::new(1)
            && *property == istr("name")
    ));
}

#[test]
fn detach_delete_node_cascades_incident_edges() {
    let graph = graph_with_edge();
    let plan = planned("MATCH (n:Victim) DETACH DELETE n FINISH");

    let (_, _) = run_write(&graph, &plan).expect("write executes");

    let snapshot = graph.read();
    assert_eq!(snapshot.node_count(), 1);
    assert_eq!(snapshot.edge_count(), 0);
}

#[test]
fn bare_delete_node_with_incident_edges_returns_g1001() {
    let graph = graph_with_edge();
    let plan = planned("MATCH (n:Victim) DELETE n FINISH");

    let err = run_write(&graph, &plan).expect_err("strict delete errors");

    assert!(matches!(
        err,
        ExecutorError::DependentObjectStillExists { .. }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::DEPENDENT_OBJECT_STILL_EXISTS);
}

#[test]
fn delete_edge_removes_edge_only() {
    let graph = graph_with_edge();
    let plan = planned("MATCH ()-[e:REL]->() DELETE e FINISH");

    let (_, _) = run_write(&graph, &plan).expect("write executes");

    let snapshot = graph.read();
    assert_eq!(snapshot.node_count(), 2);
    assert_eq!(snapshot.edge_count(), 0);
}

#[test]
fn match_after_insert_in_same_statement_sees_inserted_node() {
    let graph = empty_graph();
    let plan = planned("INSERT (n:Person {name: 'Zed'}) RETURN n.name AS name");

    let (table, _) = run_write(&graph, &plan).expect("write executes");

    assert_eq!(column_values(&table, "name"), [Value::String(istr("Zed"))]);
}

#[test]
fn mutation_without_write_txn_returns_invalid_transaction_state() {
    let graph = empty_graph();
    let plan = planned("INSERT (n:Person) RETURN n");
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    );

    let err = execute_pipeline(&plan.pipeline, seed_table(), &mut ctx).expect_err("write errors");

    assert!(matches!(err, ExecutorError::InvalidTransactionState { .. }));
    assert_eq!(err.gqlstatus(), GqlStatus::READ_ONLY_TRANSACTION_VIOLATION);
}

#[test]
fn mutator_error_surfaces_as_graph_mutation_executor_error() {
    let graph = empty_graph();
    let plan = planned("MATCH (n) SET n.age = 1 FINISH");
    let schema = BindingTableSchema {
        columns: vec![BindingTableColumn {
            name: Some(istr("n")),
            hidden: None,
            ty: AnalyzedType::Resolved(GqlType::NodeRef),
        }],
    };
    let table = BindingTable::new(
        schema,
        vec![Binding::new([Value::NodeRef(NodeId::new(999))])],
    );
    let snapshot = graph.read();
    let mut txn = graph.begin_write();
    let mut ctx = TxContext::write(
        snapshot,
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        &mut txn,
        graph.index_providers(),
    );

    let err = execute_pipeline(&plan.pipeline, table, &mut ctx).expect_err("write errors");

    assert!(matches!(err, ExecutorError::GraphMutation { .. }));
}

#[test]
fn insert_with_label_disjunction_returns_implementation_defined() {
    let graph = empty_graph();
    let plan = planned("INSERT (n:Person|Company) RETURN n");

    let err = run_write(&graph, &plan).expect_err("label expression errors");

    assert!(matches!(
        err,
        ExecutorError::ImplementationDefined {
            detail: "INSERT label expression form not implemented"
        }
    ));
}

#[test]
fn delete_path_target_returns_implementation_defined() {
    let graph = graph_with_edge();
    let plan = planned("MATCH p = (a)-[:REL]->(b) DELETE p FINISH");

    let err = run_write(&graph, &plan).expect_err("path delete errors");

    assert!(matches!(
        err,
        ExecutorError::ImplementationDefined {
            detail: "DELETE path target not implemented"
        }
    ));
}

#[test]
fn mutation_routes_through_mutator_only() {
    let graph = empty_graph();
    let plan = planned("INSERT (n:Person) FINISH");

    let (_, outcome) = run_write(&graph, &plan).expect("write executes");

    assert_eq!(outcome.changes.len(), 1);
}

#[test]
fn mutation_over_empty_input_table_produces_no_changes() {
    let graph = empty_graph();
    let plan = planned("MATCH (n:Missing) INSERT (m:New) RETURN m");

    let (table, outcome) = run_write(&graph, &plan).expect("write executes");

    assert_eq!(table.row_count(), 0);
    assert!(outcome.changes.is_empty());
}

#[test]
fn mutation_aborts_op_on_first_row_error_no_partial_rollback() {
    let graph = graph_with_person("Alice");
    let plan = planned("MATCH (n) SET n.age = 1 FINISH");
    let alice = graph
        .read()
        .node_store
        .alive
        .iter()
        .next()
        .map(|row| NodeId::new(u64::from(row) + 1))
        .expect("fixture has a node");
    let schema = BindingTableSchema {
        columns: vec![BindingTableColumn {
            name: Some(istr("n")),
            hidden: None,
            ty: AnalyzedType::Resolved(GqlType::NodeRef),
        }],
    };
    let table = BindingTable::new(
        schema,
        vec![
            Binding::new([Value::NodeRef(alice)]),
            Binding::new([Value::NodeRef(NodeId::new(999))]),
        ],
    );
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
        execute_pipeline(&plan.pipeline, table, &mut ctx)
    };

    assert!(matches!(result, Err(ExecutorError::GraphMutation { .. })));
    assert_eq!(
        txn.read().node_properties(alice).unwrap().get(&istr("age")),
        Some(&Value::Int(1))
    );
    txn.rollback();
}
