//! Executor hash join tests.

mod exec_common;

use exec_common::{ExecFixture, execute_pattern, node_ids_for, planned, props};
use selene_core::{NodeId, Value};
use selene_gql::{BuildSide, JoinTree};

#[test]
fn hash_join_emits_matched_pairs_with_left_build() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a:Person) MATCH (a)-[:KNOWS]->(b) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "a"), vec![Some(1), Some(2)]);
    assert_eq!(node_ids_for(&table, "b"), vec![Some(2), Some(4)]);
}

#[test]
fn hash_join_emits_matched_pairs_with_right_build() {
    let fixture = ExecFixture::build();
    let mut plan = planned("MATCH (a:Person) MATCH (a)-[:KNOWS]->(b) RETURN a, b");
    set_hash_build_side(
        &mut plan.pattern_plan.as_mut().expect("pattern plan").join_tree,
        BuildSide::Right,
    );
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "a"), vec![Some(1), Some(2)]);
    assert_eq!(node_ids_for(&table, "b"), vec![Some(2), Some(4)]);
}

#[test]
fn hash_join_returns_empty_when_no_matches() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a:Sensor) MATCH (a)-[:KNOWS]->(b) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert!(table.is_empty());
}

#[test]
fn hash_join_handles_multiple_matches_per_key() {
    let fixture = ExecFixture::build();
    {
        let mut txn = fixture.graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_edge(
                exec_common::istr("KNOWS"),
                NodeId::new(1),
                NodeId::new(3),
                props([(fixture.score, Value::Int(7))]),
            )
            .expect("second edge inserts");
        txn.commit().expect("fixture update commits");
    }
    let plan = planned("MATCH (a:Person) MATCH (a)-[:KNOWS]->(b) RETURN a, b");
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
        vec![(Some(1), Some(2)), (Some(1), Some(3)), (Some(2), Some(4))]
    );
}

#[test]
fn hash_join_uses_lossless_numeric_equality_in_post_join_filters() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a:Counter), (b:Counter) WHERE a.count = b.count RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);
    let mut pairs = node_ids_for(&table, "a")
        .into_iter()
        .zip(node_ids_for(&table, "b"))
        .collect::<Vec<_>>();
    pairs.sort();

    assert_eq!(pairs, vec![(Some(5), Some(5)), (Some(6), Some(6))]);
}

fn set_hash_build_side(tree: &mut JoinTree, build_side: BuildSide) {
    match tree {
        JoinTree::HashJoin {
            build_side: side, ..
        } => *side = build_side,
        other => panic!("expected HashJoin, got {other:?}"),
    }
}
