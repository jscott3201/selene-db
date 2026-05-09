//! Executor expand operator tests.

mod exec_common;

use exec_common::{ExecFixture, edge_ids_for, execute_pattern, istr, node_ids_for, planned, props};
use selene_core::{GraphId, LabelSet};
use selene_gql::{ImplDefinedCaps, TxContext};
use selene_graph::SharedGraph;

#[test]
fn expand_outgoing_emits_one_row_per_outgoing_edge() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a)-[:KNOWS]->(b) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "a"), vec![Some(1), Some(2)]);
    assert_eq!(node_ids_for(&table, "b"), vec![Some(2), Some(4)]);
}

#[test]
fn expand_incoming_emits_one_row_per_incoming_edge() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a)<-[:KNOWS]-(b) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "a"), vec![Some(2), Some(4)]);
    assert_eq!(node_ids_for(&table, "b"), vec![Some(1), Some(2)]);
}

#[test]
fn expand_both_includes_outgoing_and_incoming() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a)-[:KNOWS]-(b) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);
    let mut pairs = node_ids_for(&table, "a")
        .into_iter()
        .zip(node_ids_for(&table, "b"))
        .collect::<Vec<_>>();
    pairs.sort();

    assert_eq!(
        pairs,
        vec![
            (Some(1), Some(2)),
            (Some(2), Some(1)),
            (Some(2), Some(4)),
            (Some(4), Some(2)),
        ]
    );
}

#[test]
fn expand_both_dedups_self_loops_to_one_row() {
    let loop_label = istr("LoopNode");
    let edge_label = istr("LOOPS");
    let graph = SharedGraph::new(GraphId::new(3201));
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        let node = mutator
            .create_node(LabelSet::single(loop_label), props([]))
            .expect("node inserts");
        mutator
            .create_edge(edge_label, node, node, props([]))
            .expect("self-loop inserts");
        txn.commit().expect("fixture commits");
    }
    let plan = planned("MATCH (a:LoopNode)-[e:LOOPS]-(b) RETURN a, e, b");
    let caps = ImplDefinedCaps::default();
    let ctx = TxContext::read_only(graph.read(), &caps);
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(table.row_count(), 1);
    assert_eq!(node_ids_for(&table, "a"), vec![Some(1)]);
    assert_eq!(node_ids_for(&table, "b"), vec![Some(1)]);
    assert_eq!(edge_ids_for(&table, "e"), vec![Some(1)]);
}

#[test]
fn expand_filters_edges_by_label_predicate() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a)-[:MISSING]->(b) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert!(table.is_empty());
}

#[test]
fn expand_evaluates_edge_residual_predicates() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a)-[e:KNOWS {score: 2}]->(b) RETURN e");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(edge_ids_for(&table, "e"), vec![Some(2)]);
}

#[test]
fn expand_evaluates_right_binding_residual_predicates() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a)-[:KNOWS]->(b {score: 99}) RETURN b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "b"), vec![Some(4)]);
}

#[test]
fn expand_drops_edge_when_residual_evaluates_to_null() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a)-[e:KNOWS]->(b) WHERE e.missing RETURN e");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert!(table.is_empty());
}
