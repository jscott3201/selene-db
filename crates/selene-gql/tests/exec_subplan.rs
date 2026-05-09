//! Executor subplan tests.

mod exec_common;

use exec_common::{ExecFixture, execute_pattern, node_ids_for, planned, props};
use selene_core::{LabelSet, Value};
use selene_gql::JoinTree;

#[test]
fn subplan_recursively_executes_nested_pattern() {
    let fixture = ExecFixture::build();
    let mut plan = planned("MATCH (n:Person) RETURN n");
    let subplan = plan.clone();
    plan.pattern_plan.as_mut().expect("pattern plan").join_tree =
        JoinTree::Subplan(Box::new(subplan));
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "n"), vec![Some(1), Some(2), Some(3)]);
}

#[test]
fn subplan_uses_same_tx_context_no_new_snapshot() {
    let fixture = ExecFixture::build();
    let mut plan = planned("MATCH (n:Person) RETURN n");
    let subplan = plan.clone();
    plan.pattern_plan.as_mut().expect("pattern plan").join_tree =
        JoinTree::Subplan(Box::new(subplan));
    let ctx = fixture.context_caps(&plan);

    {
        let mut txn = fixture.graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(fixture.person),
                props([(fixture.score, Value::Int(12))]),
            )
            .expect("late node inserts");
        txn.commit().expect("late update commits");
    }

    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "n"), vec![Some(1), Some(2), Some(3)]);
}
