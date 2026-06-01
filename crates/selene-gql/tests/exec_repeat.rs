//! Executor repeat operator tests.

mod exec_common;

use exec_common::{ExecFixture, execute_pattern, execute_plan, istr, node_ids_for, planned, props};
use selene_core::{CancellationToken, GraphId, LabelSet, Value};
use selene_gql::{EmptyProcedureRegistry, ExecutorError, ImplDefinedCaps, TxContext};
use selene_graph::SharedGraph;

fn edge_lists_for(table: &selene_gql::BindingTable, name: &str) -> Vec<Option<Vec<u64>>> {
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

#[test]
fn bounded_repeat_emits_paths_across_hop_range() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a:Person)-[r:KNOWS*1..2]->(b) RETURN a, r, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "a"), vec![Some(1), Some(1), Some(2)]);
    assert_eq!(node_ids_for(&table, "b"), vec![Some(2), Some(4), Some(4)]);
    assert_eq!(
        edge_lists_for(&table, "r"),
        vec![Some(vec![1]), Some(vec![1, 2]), Some(vec![2])]
    );
}

#[test]
fn exact_repeat_emits_only_matching_depth() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a:Person)-[r:KNOWS{2}]->(b) RETURN a, r, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "a"), vec![Some(1)]);
    assert_eq!(node_ids_for(&table, "b"), vec![Some(4)]);
    assert_eq!(edge_lists_for(&table, "r"), vec![Some(vec![1, 2])]);
}

#[test]
fn zero_hop_repeat_binds_empty_edge_list_and_source_as_final_node() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a:Person)-[r:KNOWS*0..1]->(b) RETURN a, r, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(
        node_ids_for(&table, "a"),
        vec![Some(1), Some(1), Some(2), Some(2), Some(3)]
    );
    assert_eq!(
        node_ids_for(&table, "b"),
        vec![Some(1), Some(2), Some(2), Some(4), Some(3)]
    );
    assert_eq!(
        edge_lists_for(&table, "r"),
        vec![
            Some(vec![]),
            Some(vec![1]),
            Some(vec![]),
            Some(vec![2]),
            Some(vec![])
        ]
    );
}

#[test]
fn repeat_evaluates_edge_property_predicates_per_hop() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a)-[r:KNOWS*1..2 {score: 2}]->(b) RETURN r");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(edge_lists_for(&table, "r"), vec![Some(vec![2])]);
}

#[test]
fn repeat_evaluates_inline_predicates_per_hop() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a:Person)-[r:KNOWS*1..2 WHERE a.age = 42]->(b) RETURN a, r, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "a"), vec![Some(2)]);
    assert_eq!(node_ids_for(&table, "b"), vec![Some(4)]);
    assert_eq!(edge_lists_for(&table, "r"), vec![Some(vec![2])]);
}

#[test]
fn repeat_inline_predicates_see_current_group_binding() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a:Person)-[r:KNOWS*1..2 WHERE r IS NOT NULL]->(b) RETURN a, r, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "a"), vec![Some(1), Some(1), Some(2)]);
    assert_eq!(node_ids_for(&table, "b"), vec![Some(2), Some(4), Some(4)]);
    assert_eq!(
        edge_lists_for(&table, "r"),
        vec![Some(vec![1]), Some(vec![1, 2]), Some(vec![2])]
    );
}

#[test]
fn repeat_composes_under_optional_outer_join() {
    let fixture = ExecFixture::build();
    let plan =
        planned("MATCH (a:Person) OPTIONAL MATCH (a)-[r:KNOWS{2}]->(b:Person) RETURN a, b, r");

    let table = execute_plan(&fixture, &plan).expect("optional repeat executes");

    assert_eq!(node_ids_for(&table, "a"), vec![Some(1), Some(2), Some(3)]);
    assert_eq!(node_ids_for(&table, "b"), vec![None, None, None]);
    assert_eq!(edge_lists_for(&table, "r"), vec![None, None, None]);
}

#[test]
fn repeat_checks_cancellation_during_traversal() {
    let root_label = istr("Root");
    let target_label = istr("Target");
    let edge_label = istr("K");
    let graph = SharedGraph::new(GraphId::new(6201));
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        let root = mutator
            .create_node(LabelSet::single(root_label), props([]))
            .expect("root inserts");
        for _ in 0..1100 {
            let target = mutator
                .create_node(LabelSet::single(target_label.clone()), props([]))
                .expect("target inserts");
            mutator
                .create_edge(edge_label.clone(), root, target, props([]))
                .expect("edge inserts");
        }
        txn.commit().expect("fixture commits");
    }

    let plan = planned("MATCH (a:Root)-[:K*1..1]->(b) RETURN b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let caps = ImplDefinedCaps::default();
    let token = CancellationToken::new();
    token.cancel();
    let ctx = TxContext::read_only(
        graph.read(),
        &caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    )
    .with_plan_metadata(&plan.expr_ids, &plan.subqueries)
    .with_resource_limits(Some(&token), None, None);

    let err = selene_gql::execute_pattern(pattern, &ctx).expect_err("repeat observes token");

    assert!(matches!(err, ExecutorError::Cancelled { .. }));
    assert_eq!(err.gqlstatus().as_str(), "5GQL2");
}

#[test]
fn repeat_checks_cancellation_between_source_rows_without_adjacent_edges() {
    let root_label = istr("Root");
    let graph = SharedGraph::new(GraphId::new(6202));
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        for _ in 0..1100 {
            mutator
                .create_node(LabelSet::single(root_label.clone()), props([]))
                .expect("root inserts");
        }
        txn.commit().expect("fixture commits");
    }

    let plan = planned("MATCH (a:Root)-[:K*1..1]->(b) RETURN b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let caps = ImplDefinedCaps::default();
    let token = CancellationToken::new();
    token.cancel();
    let ctx = TxContext::read_only(
        graph.read(),
        &caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    )
    .with_plan_metadata(&plan.expr_ids, &plan.subqueries)
    .with_resource_limits(Some(&token), None, None);

    let err = selene_gql::execute_pattern(pattern, &ctx).expect_err("repeat observes token");

    assert!(matches!(err, ExecutorError::Cancelled { .. }));
    assert_eq!(err.gqlstatus().as_str(), "5GQL2");
}

#[test]
fn quantifier_max_exceeding_cap_is_rejected_before_execution() {
    let err = selene_gql::plan(
        &selene_gql::analyze(
            selene_gql::parse("MATCH (a)-[:K*1..101]->(b) RETURN a").expect("parses"),
            &EmptyProcedureRegistry,
            None,
        )
        .expect("analyzes"),
        &EmptyProcedureRegistry,
    )
    .expect_err("cap rejects");

    assert_eq!(err.gqlstatus().as_str(), "5GQL1");
}
