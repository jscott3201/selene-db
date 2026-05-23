//! Executor tests for unbounded and questioned quantifiers.

mod exec_common;

use exec_common::{ExecFixture, edge_ids_for, execute_plan, istr, node_ids_for, planned, props};
use selene_core::{GraphId, IStr, LabelSet, Value};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, ExecutorError, TxContext,
    execute_pattern, execute_pipeline,
};
use selene_graph::SharedGraph;

fn edge_lists_for(table: &BindingTable, name: &str) -> Vec<Option<Vec<u64>>> {
    exec_common::column_values(table, name)
        .into_iter()
        .map(|value| match value {
            Value::List(items) => Some(
                items
                    .into_iter()
                    .map(|item| match item {
                        Value::EdgeRef(id) => id.get(),
                        other => panic!("expected edge ref in group list, got {other:?}"),
                    })
                    .collect(),
            ),
            Value::Null => None,
            other => panic!("expected edge list or null, got {other:?}"),
        })
        .collect()
}

fn execute_on_graph(
    graph: &SharedGraph,
    plan: &selene_gql::ExecutionPlan,
) -> Result<BindingTable, ExecutorError> {
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    )
    .with_plan_metadata(&plan.expr_ids, &plan.subqueries);
    let input = if let Some(pattern) = &plan.pattern_plan {
        execute_pattern(pattern, &ctx)?
    } else {
        BindingTable::new(
            BindingTableSchema {
                columns: Vec::new(),
            },
            vec![Binding::empty()],
        )
    };
    execute_pipeline(&plan.pipeline, input, &mut ctx)
}

fn cycle_graph() -> SharedGraph {
    let node = istr("N");
    let edge = istr("K");
    let name = istr("name");
    let graph = SharedGraph::new(GraphId::new(6401));
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        let a = mutator
            .create_node(
                LabelSet::single(node),
                props([(name, Value::String(istr("A")))]),
            )
            .expect("A inserts");
        let b = mutator
            .create_node(
                LabelSet::single(node),
                props([(name, Value::String(istr("B")))]),
            )
            .expect("B inserts");
        mutator.create_edge(edge, a, b, props([])).expect("edge 1");
        mutator.create_edge(edge, b, a, props([])).expect("edge 2");
        txn.commit().expect("fixture commits");
    }
    graph
}

fn chain_graph() -> SharedGraph {
    let node = istr("N");
    let edge = istr("K");
    let name = istr("name");
    let graph = SharedGraph::new(GraphId::new(6402));
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        let a = named_node(&mut mutator, node, name, "A");
        let b = named_node(&mut mutator, node, name, "B");
        let c = named_node(&mut mutator, node, name, "C");
        mutator.create_edge(edge, a, b, props([])).expect("edge 1");
        mutator.create_edge(edge, b, c, props([])).expect("edge 2");
        txn.commit().expect("fixture commits");
    }
    graph
}

fn named_node(
    mutator: &mut selene_graph::Mutator<'_, '_>,
    label: IStr,
    name_key: IStr,
    name: &str,
) -> selene_core::NodeId {
    mutator
        .create_node(
            LabelSet::single(label),
            props([(name_key, Value::String(istr(name)))]),
        )
        .expect("node inserts")
}

#[test]
fn questioned_edge_emits_skipped_and_taken_rows() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a:Person {name: 'Alice'})-[r:KNOWS?]->(b) RETURN r, b");

    let table = execute_plan(&fixture, &plan).expect("questioned edge executes");

    assert_eq!(edge_ids_for(&table, "r"), vec![None, Some(1)]);
    assert_eq!(node_ids_for(&table, "b"), vec![Some(1), Some(2)]);
}

#[test]
fn questioned_edge_null_propagates_properties() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a:Person {name: 'Alice'})-[r:KNOWS?]->(b) RETURN r.score AS score");

    let table = execute_plan(&fixture, &plan).expect("questioned edge executes");

    assert_eq!(
        exec_common::column_values(&table, "score"),
        vec![Value::Null, Value::Int(1)]
    );
}

#[test]
fn questioned_edge_zero_hop_composes_with_selectors_and_path_modes() {
    let fixture = ExecFixture::build();
    let shortest = planned(
        "MATCH ANY SHORTEST (a:Person {name: 'Alice'})-[r:KNOWS?]->(b:Person {name: 'Alice'}) RETURN r, b",
    );
    let acyclic = planned(
        "MATCH ACYCLIC (a:Person {name: 'Alice'})-[r:KNOWS?]->(b:Person {name: 'Alice'}) RETURN r, b",
    );

    let shortest_rows = execute_plan(&fixture, &shortest).expect("shortest executes");
    let acyclic_rows = execute_plan(&fixture, &acyclic).expect("acyclic executes");

    assert_eq!(edge_ids_for(&shortest_rows, "r"), vec![None]);
    assert_eq!(node_ids_for(&shortest_rows, "b"), vec![Some(1)]);
    assert_eq!(edge_ids_for(&acyclic_rows, "r"), vec![None]);
    assert_eq!(node_ids_for(&acyclic_rows, "b"), vec![Some(1)]);
}

#[test]
fn unbounded_trail_prunes_repeated_edges_in_loop() {
    let graph = cycle_graph();
    let plan = planned("MATCH TRAIL (a:N {name: 'A'})-[r:K+]->(b:N) RETURN r, b");

    let table = execute_on_graph(&graph, &plan).expect("unbounded trail executes");

    assert_eq!(
        edge_lists_for(&table, "r"),
        vec![Some(vec![1]), Some(vec![1, 2])]
    );
    assert_eq!(node_ids_for(&table, "b"), vec![Some(2), Some(1)]);
}

#[test]
fn unbounded_simple_allows_terminal_return_to_source() {
    let graph = cycle_graph();
    let plan = planned("MATCH SIMPLE (a:N {name: 'A'})-[r:K+]->(b:N {name: 'A'}) RETURN r, b");

    let table = execute_on_graph(&graph, &plan).expect("unbounded simple executes");

    assert_eq!(edge_lists_for(&table, "r"), vec![Some(vec![1, 2])]);
    assert_eq!(node_ids_for(&table, "b"), vec![Some(1)]);
}

#[test]
fn unbounded_cap_exceed_returns_program_limit() {
    let graph = chain_graph();
    let mut plan = planned("MATCH ANY (a:N {name: 'A'})-[:K+]->(b:N) RETURN b");
    plan.impl_defined_caps.max_quantifier = 1;

    let err = execute_on_graph(&graph, &plan).expect_err("cap exceeds");

    assert!(matches!(
        err,
        ExecutorError::ProgramLimitExceeded {
            detail: "max_quantifier",
            ..
        }
    ));
    assert_eq!(err.gqlstatus().as_str(), "5GQL1");
}
